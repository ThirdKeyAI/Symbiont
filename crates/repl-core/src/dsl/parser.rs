//! Parser for the Symbiont DSL
//!
//! Converts a stream of tokens into an Abstract Syntax Tree (AST).

use crate::dsl::ast::*;
use crate::dsl::lexer::{Token, TokenType, Keyword};
use crate::error::{ReplError, Result};

/// Parser for the Symbiont DSL
pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
}

impl Parser {
    /// Create a new parser with the given tokens
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            current: 0,
        }
    }

    /// Parse the tokens into a program AST
    pub fn parse(&mut self) -> Result<Program> {
        let start_span = self.current_span();
        let mut declarations = Vec::new();

        // Skip initial comments and newlines
        self.skip_trivia();

        while !self.is_at_end() {
            if self.check_eof() {
                break;
            }

            match self.parse_declaration() {
                Ok(decl) => declarations.push(decl),
                Err(e) => {
                    // Error recovery: skip to next declaration
                    self.synchronize();
                    return Err(e);
                }
            }

            self.skip_trivia();
        }

        let end_span = if declarations.is_empty() {
            start_span.clone()
        } else {
            self.previous_span()
        };

        Ok(Program {
            declarations,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse a top-level declaration
    fn parse_declaration(&mut self) -> Result<Declaration> {
        if self.match_keyword(Keyword::Agent) {
            Ok(Declaration::Agent(self.parse_agent_definition()?))
        } else if self.match_keyword(Keyword::Behavior) {
            Ok(Declaration::Behavior(self.parse_behavior_definition()?))
        } else if self.match_keyword(Keyword::Function) {
            Ok(Declaration::Function(self.parse_function_definition()?))
        } else if self.match_keyword(Keyword::On) {
            Ok(Declaration::EventHandler(self.parse_event_handler()?))
        } else if self.match_keyword(Keyword::Struct) {
            Ok(Declaration::Struct(self.parse_struct_definition()?))
        } else {
            Err(ReplError::Parsing(format!(
                "Expected declaration, found {:?} at line {}",
                self.peek().token_type,
                self.peek().line
            )))
        }
    }

    /// Parse an agent definition
    fn parse_agent_definition(&mut self) -> Result<AgentDefinition> {
        let start_span = self.previous_span();
        
        let name = if let TokenType::Identifier(name) = &self.advance().token_type {
            name.clone()
        } else {
            return Err(ReplError::Parsing("Expected agent name".to_string()));
        };

        self.consume_token(TokenType::LeftBrace, "Expected '{' after agent name")?;

        let mut metadata = AgentMetadata {
            name: None,
            version: None,
            author: None,
            description: None,
        };
        let mut resources = None;
        let mut security = None;
        let mut policies = None;

        while !self.check_token(&TokenType::RightBrace) && !self.is_at_end() {
            self.skip_trivia();
            
            // Check if we've reached the end of the block after skipping trivia
            if self.check_token(&TokenType::RightBrace) {
                break;
            }

            if self.match_keyword(Keyword::Name) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'name'")?;
                metadata.name = Some(self.parse_string_literal()?);
            } else if self.match_keyword(Keyword::Version) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'version'")?;
                metadata.version = Some(self.parse_string_literal()?);
            } else if self.match_keyword(Keyword::Author) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'author'")?;
                metadata.author = Some(self.parse_string_literal()?);
            } else if self.match_keyword(Keyword::Description) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'description'")?;
                metadata.description = Some(self.parse_string_literal()?);
            } else if self.match_keyword(Keyword::Resources) {
                resources = Some(self.parse_resource_config()?);
            } else if self.match_keyword(Keyword::Security) {
                security = Some(self.parse_security_config()?);
            } else if self.match_keyword(Keyword::Policies) {
                policies = Some(self.parse_policy_config()?);
            } else {
                return Err(ReplError::Parsing(format!(
                    "Unexpected token in agent definition: {:?}",
                    self.peek().token_type
                )));
            }

            self.skip_trivia();
        }

        self.consume_token(TokenType::RightBrace, "Expected '}' after agent definition")?;

        let end_span = self.previous_span();

        Ok(AgentDefinition {
            name,
            metadata,
            resources,
            security,
            policies,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse a behavior definition
    fn parse_behavior_definition(&mut self) -> Result<BehaviorDefinition> {
        let start_span = self.previous_span();
        
        let name = if let TokenType::Identifier(name) = &self.advance().token_type {
            name.clone()
        } else {
            return Err(ReplError::Parsing("Expected behavior name".to_string()));
        };

        self.consume_token(TokenType::LeftBrace, "Expected '{' after behavior name")?;

        let mut input = None;
        let mut output = None;
        let mut steps = None;

        while !self.check_token(&TokenType::RightBrace) && !self.is_at_end() {
            self.skip_trivia();

            if self.match_keyword(Keyword::Input) {
                input = Some(self.parse_parameter_block()?);
            } else if self.match_keyword(Keyword::Output) {
                output = Some(self.parse_parameter_block()?);
            } else if self.match_keyword(Keyword::Steps) {
                steps = Some(self.parse_block()?);
            } else {
                return Err(ReplError::Parsing(format!(
                    "Unexpected token in behavior definition: {:?}",
                    self.peek().token_type
                )));
            }

            self.skip_trivia();
        }

        self.consume_token(TokenType::RightBrace, "Expected '}' after behavior definition")?;

        let steps = steps.ok_or_else(|| ReplError::Parsing("Behavior must have steps block".to_string()))?;
        let end_span = self.previous_span();

        Ok(BehaviorDefinition {
            name,
            input,
            output,
            steps,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse a function definition
    fn parse_function_definition(&mut self) -> Result<FunctionDefinition> {
        let start_span = self.previous_span();
        
        let name = if let TokenType::Identifier(name) = &self.advance().token_type {
            name.clone()
        } else {
            return Err(ReplError::Parsing("Expected function name".to_string()));
        };

        self.consume_token(TokenType::LeftParen, "Expected '(' after function name")?;
        let parameters = self.parse_parameter_list()?;
        self.consume_token(TokenType::RightParen, "Expected ')' after parameters")?;

        let return_type = if self.match_token(&TokenType::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = self.parse_block()?;
        let end_span = self.previous_span();

        Ok(FunctionDefinition {
            name,
            parameters,
            return_type,
            body,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse an event handler
    fn parse_event_handler(&mut self) -> Result<EventHandler> {
        let start_span = self.previous_span();
        
        let event_name = if let TokenType::Identifier(name) = &self.advance().token_type {
            name.clone()
        } else {
            return Err(ReplError::Parsing("Expected event name".to_string()));
        };

        self.consume_token(TokenType::LeftParen, "Expected '(' after event name")?;
        let parameters = self.parse_parameter_list()?;
        self.consume_token(TokenType::RightParen, "Expected ')' after parameters")?;

        let body = self.parse_block()?;
        let end_span = self.previous_span();

        Ok(EventHandler {
            event_name,
            parameters,
            body,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse a struct definition
    fn parse_struct_definition(&mut self) -> Result<StructDefinition> {
        let start_span = self.previous_span();
        
        let name = if let TokenType::Identifier(name) = &self.advance().token_type {
            name.clone()
        } else {
            return Err(ReplError::Parsing("Expected struct name".to_string()));
        };

        self.consume_token(TokenType::LeftBrace, "Expected '{' after struct name")?;

        let mut fields = Vec::new();
        while !self.check_token(&TokenType::RightBrace) && !self.is_at_end() {
            self.skip_trivia();
            if self.check_token(&TokenType::RightBrace) {
                break;
            }

            let field_start = self.current_span();
            let field_name = if let TokenType::Identifier(name) = &self.advance().token_type {
                name.clone()
            } else {
                return Err(ReplError::Parsing("Expected field name".to_string()));
            };

            self.consume_token(TokenType::Colon, "Expected ':' after field name")?;
            let field_type = self.parse_type()?;
            let field_end = self.previous_span();

            fields.push(StructField {
                name: field_name,
                field_type,
                span: Span {
                    start: field_start.start,
                    end: field_end.end,
                },
            });

            self.skip_trivia();
        }

        self.consume_token(TokenType::RightBrace, "Expected '}' after struct fields")?;
        let end_span = self.previous_span();

        Ok(StructDefinition {
            name,
            fields,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse a block of statements
    fn parse_block(&mut self) -> Result<Block> {
        let start_span = self.current_span();
        self.consume_token(TokenType::LeftBrace, "Expected '{'")?;

        let mut statements = Vec::new();
        while !self.check_token(&TokenType::RightBrace) && !self.is_at_end() {
            self.skip_trivia();
            if self.check_token(&TokenType::RightBrace) {
                break;
            }

            statements.push(self.parse_statement()?);
            self.skip_trivia();
        }

        self.consume_token(TokenType::RightBrace, "Expected '}'")?;
        let end_span = self.previous_span();

        Ok(Block {
            statements,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse a statement
    fn parse_statement(&mut self) -> Result<Statement> {
        if self.match_keyword(Keyword::Let) {
            Ok(Statement::Let(self.parse_let_statement()?))
        } else if self.match_keyword(Keyword::If) {
            Ok(Statement::If(self.parse_if_statement()?))
        } else if self.match_keyword(Keyword::Return) {
            Ok(Statement::Return(self.parse_return_statement()?))
        } else if self.match_keyword(Keyword::Emit) {
            Ok(Statement::Emit(self.parse_emit_statement()?))
        } else if self.match_keyword(Keyword::Require) {
            Ok(Statement::Require(self.parse_require_statement()?))
        } else {
            // Expression statement
            let expr = self.parse_expression()?;
            Ok(Statement::Expression(expr))
        }
    }

    /// Parse a let statement
    fn parse_let_statement(&mut self) -> Result<LetStatement> {
        let start_span = self.previous_span();
        
        let name = if let TokenType::Identifier(name) = &self.advance().token_type {
            name.clone()
        } else {
            return Err(ReplError::Parsing("Expected variable name".to_string()));
        };

        let var_type = if self.match_token(&TokenType::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        self.consume_token(TokenType::Assign, "Expected '=' in let statement")?;
        let value = self.parse_expression()?;
        let end_span = self.previous_span();

        Ok(LetStatement {
            name,
            var_type,
            value,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse an if statement
    fn parse_if_statement(&mut self) -> Result<IfStatement> {
        let start_span = self.previous_span();
        
        let condition = self.parse_expression()?;
        let then_block = self.parse_block()?;

        let mut else_ifs = Vec::new();
        while self.match_keyword(Keyword::Else) {
            if self.match_keyword(Keyword::If) {
                let else_if_start = self.previous_span();
                let else_if_condition = self.parse_expression()?;
                let else_if_block = self.parse_block()?;
                let else_if_end = self.previous_span();

                else_ifs.push(ElseIf {
                    condition: else_if_condition,
                    block: else_if_block,
                    span: Span {
                        start: else_if_start.start,
                        end: else_if_end.end,
                    },
                });
            } else {
                // Final else block
                let else_block = self.parse_block()?;
                let end_span = self.previous_span();

                return Ok(IfStatement {
                    condition,
                    then_block,
                    else_ifs,
                    else_block: Some(else_block),
                    span: Span {
                        start: start_span.start,
                        end: end_span.end,
                    },
                });
            }
        }

        let end_span = self.previous_span();

        Ok(IfStatement {
            condition,
            then_block,
            else_ifs,
            else_block: None,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse a return statement
    fn parse_return_statement(&mut self) -> Result<ReturnStatement> {
        let start_span = self.previous_span();
        
        let value = if self.check_token(&TokenType::Newline) || 
                      self.check_token(&TokenType::RightBrace) ||
                      self.is_at_end() {
            None
        } else {
            Some(self.parse_expression()?)
        };

        let end_span = self.previous_span();

        Ok(ReturnStatement {
            value,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse an emit statement
    fn parse_emit_statement(&mut self) -> Result<EmitStatement> {
        let start_span = self.previous_span();
        
        let event_name = if let TokenType::Identifier(name) = &self.advance().token_type {
            name.clone()
        } else {
            return Err(ReplError::Parsing("Expected event name".to_string()));
        };

        let data = if self.check_token(&TokenType::LeftBrace) {
            Some(self.parse_expression()?)
        } else {
            None
        };

        let end_span = self.previous_span();

        Ok(EmitStatement {
            event_name,
            data,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse a require statement
    fn parse_require_statement(&mut self) -> Result<RequireStatement> {
        let start_span = self.previous_span();
        
        let requirement = if self.match_keyword(Keyword::Capability) {
            let cap_name = if let TokenType::Identifier(name) = &self.advance().token_type {
                name.clone()
            } else {
                return Err(ReplError::Parsing("Expected capability name".to_string()));
            };
            RequirementType::Capability(cap_name)
        } else if self.match_keyword(Keyword::Capabilities) {
            self.consume_token(TokenType::LeftBracket, "Expected '[' after 'capabilities'")?;
            let mut capabilities = Vec::new();
            
            while !self.check_token(&TokenType::RightBracket) && !self.is_at_end() {
                if let TokenType::Identifier(name) = &self.advance().token_type {
                    capabilities.push(name.clone());
                } else {
                    return Err(ReplError::Parsing("Expected capability name".to_string()));
                }

                if !self.check_token(&TokenType::RightBracket) {
                    self.consume_token(TokenType::Comma, "Expected ',' between capabilities")?;
                }
            }

            self.consume_token(TokenType::RightBracket, "Expected ']' after capabilities")?;
            RequirementType::Capabilities(capabilities)
        } else {
            return Err(ReplError::Parsing("Expected 'capability' or 'capabilities'".to_string()));
        };

        let end_span = self.previous_span();

        Ok(RequireStatement {
            requirement,
            span: Span {
                start: start_span.start,
                end: end_span.end,
            },
        })
    }

    /// Parse an expression
    fn parse_expression(&mut self) -> Result<Expression> {
        self.parse_assignment()
    }

    /// Parse assignment expression
    fn parse_assignment(&mut self) -> Result<Expression> {
        let expr = self.parse_logical_or()?;

        if self.match_token(&TokenType::Assign) {
            let start_span = self.get_expression_span(&expr);
            let value = self.parse_assignment()?;
            let end_span = self.get_expression_span(&value);

            return Ok(Expression::Assignment(Assignment {
                target: Box::new(expr),
                value: Box::new(value),
                span: Span {
                    start: start_span.start,
                    end: end_span.end,
                },
            }));
        }

        Ok(expr)
    }

    /// Parse logical OR expression
    fn parse_logical_or(&mut self) -> Result<Expression> {
        let mut expr = self.parse_logical_and()?;

        while self.match_token(&TokenType::Or) {
            let start_span = self.get_expression_span(&expr);
            let operator = BinaryOperator::Or;
            let right = self.parse_logical_and()?;
            let end_span = self.get_expression_span(&right);

            expr = Expression::BinaryOp(BinaryOperation {
                left: Box::new(expr),
                operator,
                right: Box::new(right),
                span: Span {
                    start: start_span.start,
                    end: end_span.end,
                },
            });
        }

        Ok(expr)
    }

    /// Parse logical AND expression
    fn parse_logical_and(&mut self) -> Result<Expression> {
        let mut expr = self.parse_equality()?;

        while self.match_token(&TokenType::And) {
            let start_span = self.get_expression_span(&expr);
            let operator = BinaryOperator::And;
            let right = self.parse_equality()?;
            let end_span = self.get_expression_span(&right);

            expr = Expression::BinaryOp(BinaryOperation {
                left: Box::new(expr),
                operator,
                right: Box::new(right),
                span: Span {
                    start: start_span.start,
                    end: end_span.end,
                },
            });
        }

        Ok(expr)
    }

    /// Parse equality expression
    fn parse_equality(&mut self) -> Result<Expression> {
        let mut expr = self.parse_comparison()?;

        while let Some(op) = self.match_binary_operator(&[TokenType::Equal, TokenType::NotEqual]) {
            let start_span = self.get_expression_span(&expr);
            let right = self.parse_comparison()?;
            let end_span = self.get_expression_span(&right);

            expr = Expression::BinaryOp(BinaryOperation {
                left: Box::new(expr),
                operator: op,
                right: Box::new(right),
                span: Span {
                    start: start_span.start,
                    end: end_span.end,
                },
            });
        }

        Ok(expr)
    }

    /// Parse comparison expression
    fn parse_comparison(&mut self) -> Result<Expression> {
        let mut expr = self.parse_addition()?;

        while let Some(op) = self.match_binary_operator(&[
            TokenType::GreaterThan,
            TokenType::GreaterThanOrEqual,
            TokenType::LessThan,
            TokenType::LessThanOrEqual,
        ]) {
            let start_span = self.get_expression_span(&expr);
            let right = self.parse_addition()?;
            let end_span = self.get_expression_span(&right);

            expr = Expression::BinaryOp(BinaryOperation {
                left: Box::new(expr),
                operator: op,
                right: Box::new(right),
                span: Span {
                    start: start_span.start,
                    end: end_span.end,
                },
            });
        }

        Ok(expr)
    }

    /// Parse addition/subtraction expression
    fn parse_addition(&mut self) -> Result<Expression> {
        let mut expr = self.parse_multiplication()?;

        while let Some(op) = self.match_binary_operator(&[TokenType::Plus, TokenType::Minus]) {
            let start_span = self.get_expression_span(&expr);
            let right = self.parse_multiplication()?;
            let end_span = self.get_expression_span(&right);

            expr = Expression::BinaryOp(BinaryOperation {
                left: Box::new(expr),
                operator: op,
                right: Box::new(right),
                span: Span {
                    start: start_span.start,
                    end: end_span.end,
                },
            });
        }

        Ok(expr)
    }

    /// Parse multiplication/division expression
    fn parse_multiplication(&mut self) -> Result<Expression> {
        let mut expr = self.parse_unary()?;

        while let Some(op) = self.match_binary_operator(&[
            TokenType::Multiply,
            TokenType::Divide,
            TokenType::Modulo,
        ]) {
            let start_span = self.get_expression_span(&expr);
            let right = self.parse_unary()?;
            let end_span = self.get_expression_span(&right);

            expr = Expression::BinaryOp(BinaryOperation {
                left: Box::new(expr),
                operator: op,
                right: Box::new(right),
                span: Span {
                    start: start_span.start,
                    end: end_span.end,
                },
            });
        }

        Ok(expr)
    }

    /// Parse unary expression
    fn parse_unary(&mut self) -> Result<Expression> {
        if let Some(op) = self.match_unary_operator(&[TokenType::Not, TokenType::Minus]) {
            let start_span = self.previous_span();
            let operand = self.parse_unary()?;
            let end_span = self.get_expression_span(&operand);

            return Ok(Expression::UnaryOp(UnaryOperation {
                operator: op,
                operand: Box::new(operand),
                span: Span {
                    start: start_span.start,
                    end: end_span.end,
                },
            }));
        }

        self.parse_postfix()
    }

    /// Parse postfix expression (field access, indexing, method calls)
    fn parse_postfix(&mut self) -> Result<Expression> {
        let mut expr = self.parse_primary()?;

        loop {
            if self.match_token(&TokenType::Dot) {
                let start_span = self.get_expression_span(&expr);
                let field = if let TokenType::Identifier(name) = &self.advance().token_type {
                    name.clone()
                } else {
                    return Err(ReplError::Parsing("Expected field name after '.'".to_string()));
                };

                // Check if this is a method call
                if self.check_token(&TokenType::LeftParen) {
                    self.advance(); // consume '('
                    let arguments = self.parse_argument_list()?;
                    self.consume_token(TokenType::RightParen, "Expected ')' after arguments")?;
                    let end_span = self.previous_span();

                    expr = Expression::MethodCall(MethodCall {
                        object: Box::new(expr),
                        method: field,
                        arguments,
                        span: Span {
                            start: start_span.start,
                            end: end_span.end,
                        },
                    });
                } else {
                    let end_span = self.previous_span();

                    expr = Expression::FieldAccess(FieldAccess {
                        object: Box::new(expr),
                        field,
                        span: Span {
                            start: start_span.start,
                            end: end_span.end,
                        },
                    });
                }
            } else if self.match_token(&TokenType::LeftBracket) {
                let start_span = self.get_expression_span(&expr);
                let index = self.parse_expression()?;
                self.consume_token(TokenType::RightBracket, "Expected ']' after index")?;
                let end_span = self.previous_span();

                expr = Expression::IndexAccess(IndexAccess {
                    object: Box::new(expr),
                    index: Box::new(index),
                    span: Span {
                        start: start_span.start,
                        end: end_span.end,
                    },
                });
            } else {
                break;
            }
        }

        Ok(expr)
    }

    /// Parse primary expression
    fn parse_primary(&mut self) -> Result<Expression> {
        let token = self.advance();
        let span = self.token_span(&token);

        match &token.token_type {
            TokenType::String(s) => Ok(Expression::Literal(Literal::String(s.clone()))),
            TokenType::Number(n) => Ok(Expression::Literal(Literal::Number(*n))),
            TokenType::Integer(i) => Ok(Expression::Literal(Literal::Integer(*i))),
            TokenType::Duration(value, unit) => {
                let duration_unit = match unit.as_str() {
                    "ms" => DurationUnit::Milliseconds,
                    "s" => DurationUnit::Seconds,
                    "m" => DurationUnit::Minutes,
                    "h" => DurationUnit::Hours,
                    "d" => DurationUnit::Days,
                    _ => return Err(ReplError::Parsing(format!("Invalid duration unit: {}", unit))),
                };
                Ok(Expression::Literal(Literal::Duration(DurationValue {
                    value: *value,
                    unit: duration_unit,
                })))
            }
            TokenType::Size(value, unit) => {
                let size_unit = match unit.as_str() {
                    "B" => SizeUnit::Bytes,
                    "KB" => SizeUnit::KB,
                    "MB" => SizeUnit::MB,
                    "GB" => SizeUnit::GB,
                    "TB" => SizeUnit::TB,
                    _ => return Err(ReplError::Parsing(format!("Invalid size unit: {}", unit))),
                };
                Ok(Expression::Literal(Literal::Size(SizeValue {
                    value: *value,
                    unit: size_unit,
                })))
            }
            TokenType::Keyword(Keyword::True) => Ok(Expression::Literal(Literal::Boolean(true))),
            TokenType::Keyword(Keyword::False) => Ok(Expression::Literal(Literal::Boolean(false))),
            TokenType::Keyword(Keyword::Null) => Ok(Expression::Literal(Literal::Null)),
            TokenType::Identifier(name) => {
                // Check for function call
                if self.check_token(&TokenType::LeftParen) {
                    self.advance(); // consume '('
                    let arguments = self.parse_argument_list()?;
                    self.consume_token(TokenType::RightParen, "Expected ')' after arguments")?;
                    let end_span = self.previous_span();

                    Ok(Expression::FunctionCall(FunctionCall {
                        function: name.clone(),
                        arguments,
                        span: Span {
                            start: span.start,
                            end: end_span.end,
                        },
                    }))
                } else {
                    Ok(Expression::Identifier(Identifier {
                        name: name.clone(),
                        span,
                    }))
                }
            }
            TokenType::LeftParen => {
                let expr = self.parse_expression()?;
                self.consume_token(TokenType::RightParen, "Expected ')' after expression")?;
                Ok(expr)
            }
            TokenType::LeftBracket => {
                let elements = self.parse_expression_list(&TokenType::RightBracket)?;
                self.consume_token(TokenType::RightBracket, "Expected ']' after list elements")?;
                let end_span = self.previous_span();

                Ok(Expression::List(ListExpression {
                    elements,
                    span: Span {
                        start: span.start,
                        end: end_span.end,
                    },
                }))
            }
            TokenType::LeftBrace => {
                let entries = self.parse_map_entries()?;
                self.consume_token(TokenType::RightBrace, "Expected '}' after map entries")?;
                let end_span = self.previous_span();

                Ok(Expression::Map(MapExpression {
                    entries,
                    span: Span {
                        start: span.start,
                        end: end_span.end,
                    },
                }))
            }
            _ => Err(ReplError::Parsing(format!(
                "Unexpected token in expression: {:?}",
                token.token_type
            ))),
        }
    }

    // Helper methods for parsing configuration blocks
    fn parse_resource_config(&mut self) -> Result<ResourceConfig> {
        self.consume_token(TokenType::LeftBrace, "Expected '{' after 'resources'")?;

        let mut memory = None;
        let mut cpu = None;
        let mut network = None;
        let mut storage = None;

        while !self.check_token(&TokenType::RightBrace) && !self.is_at_end() {
            self.skip_trivia();

            if self.match_keyword(Keyword::Memory) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'memory'")?;
                memory = Some(self.parse_size_value()?);
            } else if self.match_keyword(Keyword::Cpu) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'cpu'")?;
                cpu = Some(self.parse_duration_value()?);
            } else if self.match_keyword(Keyword::Network) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'network'")?;
                network = Some(self.parse_boolean_value()?);
            } else if self.match_keyword(Keyword::Storage) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'storage'")?;
                storage = Some(self.parse_size_value()?);
            } else {
                return Err(ReplError::Parsing("Unexpected token in resources block".to_string()));
            }

            self.skip_trivia();
        }

        self.consume_token(TokenType::RightBrace, "Expected '}' after resources")?;

        Ok(ResourceConfig {
            memory,
            cpu,
            network,
            storage,
        })
    }

    fn parse_security_config(&mut self) -> Result<SecurityConfig> {
        self.consume_token(TokenType::LeftBrace, "Expected '{' after 'security'")?;

        let mut tier = None;
        let mut capabilities = Vec::new();
        let mut sandbox = None;

        while !self.check_token(&TokenType::RightBrace) && !self.is_at_end() {
            self.skip_trivia();

            if self.match_keyword(Keyword::Tier) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'tier'")?;
                tier = Some(self.parse_security_tier()?);
            } else if self.match_keyword(Keyword::Capabilities) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'capabilities'")?;
                capabilities = self.parse_string_list()?;
            } else if self.match_keyword(Keyword::Sandbox) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'sandbox'")?;
                sandbox = Some(self.parse_sandbox_mode()?);
            } else {
                return Err(ReplError::Parsing("Unexpected token in security block".to_string()));
            }

            self.skip_trivia();
        }

        self.consume_token(TokenType::RightBrace, "Expected '}' after security")?;

        Ok(SecurityConfig {
            tier,
            capabilities,
            sandbox,
        })
    }

    fn parse_policy_config(&mut self) -> Result<PolicyConfig> {
        self.consume_token(TokenType::LeftBrace, "Expected '{' after 'policies'")?;

        let mut execution_timeout = None;
        let mut retry_count = None;
        let mut failure_action = None;

        while !self.check_token(&TokenType::RightBrace) && !self.is_at_end() {
            self.skip_trivia();

            if self.match_keyword(Keyword::Timeout) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'timeout'")?;
                execution_timeout = Some(self.parse_duration_value()?);
            } else if self.match_keyword(Keyword::Retry) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'retry'")?;
                retry_count = Some(self.parse_integer_value()?);
            } else if self.match_keyword(Keyword::Failure) {
                self.consume_token(TokenType::Colon, "Expected ':' after 'failure'")?;
                failure_action = Some(self.parse_failure_action()?);
            } else {
                return Err(ReplError::Parsing("Unexpected token in policies block".to_string()));
            }

            self.skip_trivia();
        }

        self.consume_token(TokenType::RightBrace, "Expected '}' after policies")?;

        Ok(PolicyConfig {
            execution_timeout,
            retry_count,
            failure_action,
        })
    }

    fn parse_parameter_block(&mut self) -> Result<ParameterList> {
        self.consume_token(TokenType::LeftBrace, "Expected '{' after parameter block")?;
        let parameters = self.parse_parameter_list()?;
        self.consume_token(TokenType::RightBrace, "Expected '}' after parameter block")?;
        Ok(parameters)
    }

    fn parse_parameter_list(&mut self) -> Result<ParameterList> {
        let mut parameters = Vec::new();

        while !self.check_token(&TokenType::RightParen) && 
              !self.check_token(&TokenType::RightBrace) && 
              !self.is_at_end() {
            self.skip_trivia();
            
            if self.check_token(&TokenType::RightParen) || 
               self.check_token(&TokenType::RightBrace) {
                break;
            }

            let param_start = self.current_span();
            let name = if let TokenType::Identifier(name) = &self.advance().token_type {
                name.clone()
            } else {
                return Err(ReplError::Parsing("Expected parameter name".to_string()));
            };

            self.consume_token(TokenType::Colon, "Expected ':' after parameter name")?;
            let param_type = self.parse_type()?;

            let default_value = if self.match_token(&TokenType::Assign) {
                Some(self.parse_expression()?)
            } else {
                None
            };

            let param_end = self.previous_span();

            parameters.push(Parameter {
                name,
                param_type,
                default_value,
                span: Span {
                    start: param_start.start,
                    end: param_end.end,
                },
            });

            self.skip_trivia();
            
            if !self.check_token(&TokenType::RightParen) &&
               !self.check_token(&TokenType::RightBrace) {
                self.consume_token(TokenType::Comma, "Expected ',' between parameters")?;
                self.skip_trivia();
            }
        }

        Ok(ParameterList { parameters })
    }

    fn parse_type(&mut self) -> Result<Type> {
        if let TokenType::Identifier(name) = &self.advance().token_type {
            match name.as_str() {
                "string" => Ok(Type::String),
                "number" => Ok(Type::Number),
                "boolean" => Ok(Type::Boolean),
                "datetime" => Ok(Type::DateTime),
                "duration" => Ok(Type::Duration),
                "size" => Ok(Type::Size),
                "any" => Ok(Type::Any),
                "list" => {
                    if self.match_token(&TokenType::LessThan) {
                        let inner_type = self.parse_type()?;
                        self.consume_token(TokenType::GreaterThan, "Expected '>' after list type")?;
                        Ok(Type::List(Box::new(inner_type)))
                    } else {
                        Ok(Type::List(Box::new(Type::Any)))
                    }
                }
                "map" => {
                    if self.match_token(&TokenType::LessThan) {
                        let key_type = self.parse_type()?;
                        self.consume_token(TokenType::Comma, "Expected ',' between map types")?;
                        let value_type = self.parse_type()?;
                        self.consume_token(TokenType::GreaterThan, "Expected '>' after map type")?;
                        Ok(Type::Map(Box::new(key_type), Box::new(value_type)))
                    } else {
                        Ok(Type::Map(Box::new(Type::String), Box::new(Type::Any)))
                    }
                }
                _ => Ok(Type::Custom(name.clone())),
            }
        } else {
            Err(ReplError::Parsing("Expected type name".to_string()))
        }
    }

    // Helper parsing methods
    fn parse_string_literal(&mut self) -> Result<String> {
        if let TokenType::String(s) = &self.advance().token_type {
            Ok(s.clone())
        } else {
            Err(ReplError::Parsing("Expected string literal".to_string()))
        }
    }

    fn parse_size_value(&mut self) -> Result<SizeValue> {
        if let TokenType::Size(value, unit) = &self.advance().token_type {
            let size_unit = match unit.as_str() {
                "B" => SizeUnit::Bytes,
                "KB" => SizeUnit::KB,
                "MB" => SizeUnit::MB,
                "GB" => SizeUnit::GB,
                "TB" => SizeUnit::TB,
                _ => return Err(ReplError::Parsing(format!("Invalid size unit: {}", unit))),
            };
            Ok(SizeValue {
                value: *value,
                unit: size_unit,
            })
        } else {
            Err(ReplError::Parsing("Expected size value".to_string()))
        }
    }

    fn parse_duration_value(&mut self) -> Result<DurationValue> {
        if let TokenType::Duration(value, unit) = &self.advance().token_type {
            let duration_unit = match unit.as_str() {
                "ms" => DurationUnit::Milliseconds,
                "s" => DurationUnit::Seconds,
                "m" => DurationUnit::Minutes,
                "h" => DurationUnit::Hours,
                "d" => DurationUnit::Days,
                _ => return Err(ReplError::Parsing(format!("Invalid duration unit: {}", unit))),
            };
            Ok(DurationValue {
                value: *value,
                unit: duration_unit,
            })
        } else {
            Err(ReplError::Parsing("Expected duration value".to_string()))
        }
    }

    fn parse_boolean_value(&mut self) -> Result<bool> {
        match &self.advance().token_type {
            TokenType::Keyword(Keyword::True) => Ok(true),
            TokenType::Keyword(Keyword::False) => Ok(false),
            TokenType::Keyword(Keyword::Allow) => Ok(true),
            _ => Err(ReplError::Parsing("Expected boolean value".to_string())),
        }
    }

    fn parse_integer_value(&mut self) -> Result<u32> {
        if let TokenType::Integer(i) = &self.advance().token_type {
            Ok(*i as u32)
        } else {
            Err(ReplError::Parsing("Expected integer value".to_string()))
        }
    }

    fn parse_security_tier(&mut self) -> Result<SecurityTier> {
        match &self.advance().token_type {
            TokenType::Keyword(Keyword::Tier1) => Ok(SecurityTier::Tier1),
            TokenType::Keyword(Keyword::Tier2) => Ok(SecurityTier::Tier2),
            TokenType::Keyword(Keyword::Tier3) => Ok(SecurityTier::Tier3),
            TokenType::Keyword(Keyword::Tier4) => Ok(SecurityTier::Tier4),
            _ => Err(ReplError::Parsing("Expected security tier".to_string())),
        }
    }

    fn parse_sandbox_mode(&mut self) -> Result<SandboxMode> {
        match &self.advance().token_type {
            TokenType::Keyword(Keyword::Strict) => Ok(SandboxMode::Strict),
            TokenType::Keyword(Keyword::Moderate) => Ok(SandboxMode::Moderate),
            TokenType::Keyword(Keyword::Permissive) => Ok(SandboxMode::Permissive),
            _ => Err(ReplError::Parsing("Expected sandbox mode".to_string())),
        }
    }

    fn parse_failure_action(&mut self) -> Result<FailureAction> {
        match &self.advance().token_type {
            TokenType::Keyword(Keyword::Terminate) => Ok(FailureAction::Terminate),
            TokenType::Keyword(Keyword::Restart) => Ok(FailureAction::Restart),
            TokenType::Keyword(Keyword::Escalate) => Ok(FailureAction::Escalate),
            TokenType::Keyword(Keyword::Ignore) => Ok(FailureAction::Ignore),
            _ => Err(ReplError::Parsing("Expected failure action".to_string())),
        }
    }

    fn parse_string_list(&mut self) -> Result<Vec<String>> {
        self.consume_token(TokenType::LeftBracket, "Expected '[' for string list")?;
        
        let mut strings = Vec::new();
        while !self.check_token(&TokenType::RightBracket) && !self.is_at_end() {
            strings.push(self.parse_string_literal()?);
            
            if !self.check_token(&TokenType::RightBracket) {
                self.consume_token(TokenType::Comma, "Expected ',' between list items")?;
            }
        }
        
        self.consume_token(TokenType::RightBracket, "Expected ']' after string list")?;
        Ok(strings)
    }

    fn parse_argument_list(&mut self) -> Result<Vec<Expression>> {
        let mut arguments = Vec::new();

        while !self.check_token(&TokenType::RightParen) && !self.is_at_end() {
            arguments.push(self.parse_expression()?);
            
            if !self.check_token(&TokenType::RightParen) {
                self.consume_token(TokenType::Comma, "Expected ',' between arguments")?;
            }
        }

        Ok(arguments)
    }

    fn parse_expression_list(&mut self, terminator: &TokenType) -> Result<Vec<Expression>> {
        let mut expressions = Vec::new();

        while !self.check_token(terminator) && !self.is_at_end() {
            expressions.push(self.parse_expression()?);
            
            if !self.check_token(terminator) {
                self.consume_token(TokenType::Comma, "Expected ',' between expressions")?;
            }
        }

        Ok(expressions)
    }

    fn parse_map_entries(&mut self) -> Result<Vec<MapEntry>> {
        let mut entries = Vec::new();

        while !self.check_token(&TokenType::RightBrace) && !self.is_at_end() {
            let entry_start = self.current_span();
            let key = self.parse_expression()?;
            self.consume_token(TokenType::Colon, "Expected ':' after map key")?;
            let value = self.parse_expression()?;
            let entry_end = self.previous_span();

            entries.push(MapEntry {
                key,
                value,
                span: Span {
                    start: entry_start.start,
                    end: entry_end.end,
                },
            });

            if !self.check_token(&TokenType::RightBrace) {
                self.consume_token(TokenType::Comma, "Expected ',' between map entries")?;
            }
        }

        Ok(entries)
    }

    // Utility methods
    fn match_keyword(&mut self, keyword: Keyword) -> bool {
        if let TokenType::Keyword(k) = &self.peek().token_type {
            if *k == keyword {
                self.advance();
                return true;
            }
        }
        false
    }

    fn match_token(&mut self, token_type: &TokenType) -> bool {
        if std::mem::discriminant(&self.peek().token_type) == std::mem::discriminant(token_type) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn match_binary_operator(&mut self, operators: &[TokenType]) -> Option<BinaryOperator> {
        for op_token in operators {
            if std::mem::discriminant(&self.peek().token_type) == std::mem::discriminant(op_token) {
                self.advance();
                return Some(self.token_to_binary_operator(op_token));
            }
        }
        None
    }

    fn match_unary_operator(&mut self, operators: &[TokenType]) -> Option<UnaryOperator> {
        for op_token in operators {
            if std::mem::discriminant(&self.peek().token_type) == std::mem::discriminant(op_token) {
                self.advance();
                return Some(self.token_to_unary_operator(op_token));
            }
        }
        None
    }

    fn token_to_binary_operator(&self, token: &TokenType) -> BinaryOperator {
        match token {
            TokenType::Plus => BinaryOperator::Add,
            TokenType::Minus => BinaryOperator::Subtract,
            TokenType::Multiply => BinaryOperator::Multiply,
            TokenType::Divide => BinaryOperator::Divide,
            TokenType::Modulo => BinaryOperator::Modulo,
            TokenType::Equal => BinaryOperator::Equal,
            TokenType::NotEqual => BinaryOperator::NotEqual,
            TokenType::LessThan => BinaryOperator::LessThan,
            TokenType::LessThanOrEqual => BinaryOperator::LessThanOrEqual,
            TokenType::GreaterThan => BinaryOperator::GreaterThan,
            TokenType::GreaterThanOrEqual => BinaryOperator::GreaterThanOrEqual,
            TokenType::And => BinaryOperator::And,
            TokenType::Or => BinaryOperator::Or,
            _ => panic!("Invalid binary operator token: {:?}", token),
        }
    }

    fn token_to_unary_operator(&self, token: &TokenType) -> UnaryOperator {
        match token {
            TokenType::Not => UnaryOperator::Not,
            TokenType::Minus => UnaryOperator::Negate,
            _ => panic!("Invalid unary operator token: {:?}", token),
        }
    }

    fn consume_token(&mut self, expected: TokenType, message: &str) -> Result<Token> {
        if std::mem::discriminant(&self.peek().token_type) == std::mem::discriminant(&expected) {
            Ok(self.advance())
        } else {
            Err(ReplError::Parsing(format!(
                "{}: expected {:?}, found {:?} at line {}",
                message,
                expected,
                self.peek().token_type,
                self.peek().line
            )))
        }
    }

    fn check_token(&self, token_type: &TokenType) -> bool {
        std::mem::discriminant(&self.peek().token_type) == std::mem::discriminant(token_type)
    }

    fn check_eof(&self) -> bool {
        matches!(self.peek().token_type, TokenType::Eof)
    }

    fn is_at_end(&self) -> bool {
        self.current >= self.tokens.len() || matches!(self.peek().token_type, TokenType::Eof)
    }

    fn peek(&self) -> Token {
        if self.current < self.tokens.len() {
            self.tokens[self.current].clone()
        } else {
            // Return a default EOF token if we're past the end
            Token {
                token_type: TokenType::Eof,
                line: if self.tokens.is_empty() { 1 } else { self.tokens.last().unwrap().line },
                column: 0,
                offset: if self.tokens.is_empty() { 0 } else { self.tokens.last().unwrap().offset },
                length: 0,
            }
        }
    }

    fn advance(&mut self) -> Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    fn previous(&self) -> Token {
        if self.current > 0 {
            self.tokens[self.current - 1].clone()
        } else {
            self.tokens[0].clone()
        }
    }

    fn skip_trivia(&mut self) {
        while !self.is_at_end() {
            match &self.peek().token_type {
                TokenType::Newline | TokenType::Comment(_) => {
                    self.advance();
                }
                _ => break,
            }
        }
    }

    fn synchronize(&mut self) {
        self.advance();

        while !self.is_at_end() {
            if matches!(self.previous().token_type, TokenType::Semicolon | TokenType::Newline) {
                return;
            }

            match &self.peek().token_type {
                TokenType::Keyword(Keyword::Agent) |
                TokenType::Keyword(Keyword::Behavior) |
                TokenType::Keyword(Keyword::Function) |
                TokenType::Keyword(Keyword::Struct) |
                TokenType::Keyword(Keyword::Let) |
                TokenType::Keyword(Keyword::If) |
                TokenType::Keyword(Keyword::Return) => return,
                _ => {}
            }

            self.advance();
        }
    }

    // Span helper methods
    fn current_span(&self) -> Span {
        let token = self.peek();
        self.token_span(&token)
    }

    fn previous_span(&self) -> Span {
        let token = self.previous();
        self.token_span(&token)
    }

    fn token_span(&self, token: &Token) -> Span {
        Span {
            start: SourceLocation {
                line: token.line,
                column: token.column,
                offset: token.offset,
            },
            end: SourceLocation {
                line: token.line,
                column: token.column + token.length,
                offset: token.offset + token.length,
            },
        }
    }

    fn get_expression_span(&self, expr: &Expression) -> Span {
        match expr {
            Expression::Literal(_) => self.current_span(),
            Expression::Identifier(id) => id.span.clone(),
            Expression::FieldAccess(fa) => fa.span.clone(),
            Expression::IndexAccess(ia) => ia.span.clone(),
            Expression::FunctionCall(fc) => fc.span.clone(),
            Expression::MethodCall(mc) => mc.span.clone(),
            Expression::BinaryOp(bo) => bo.span.clone(),
            Expression::UnaryOp(uo) => uo.span.clone(),
            Expression::Assignment(a) => a.span.clone(),
            Expression::List(l) => l.span.clone(),
            Expression::Map(m) => m.span.clone(),
            Expression::Invoke(i) => i.span.clone(),
            Expression::Lambda(l) => l.span.clone(),
            Expression::Conditional(c) => c.span.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::lexer::Lexer;

    fn parse_source(source: &str) -> Result<Program> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        parser.parse()
    }

    #[test]
    fn test_simple_agent() {
        let source = r#"
        agent TestAgent {
            name: "Test Agent"
            version: "1.0.0"
        }
        "#;

        let program = parse_source(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
        
        if let Declaration::Agent(agent) = &program.declarations[0] {
            assert_eq!(agent.name, "TestAgent");
            assert_eq!(agent.metadata.name, Some("Test Agent".to_string()));
            assert_eq!(agent.metadata.version, Some("1.0.0".to_string()));
        } else {
            panic!("Expected agent declaration");
        }
    }

    #[test]
    fn test_simple_behavior() {
        let source = r#"
        behavior ProcessData {
            input {
                data: string
            }
            output {
                result: string
            }
            steps {
                let processed = data.upper()
                return processed
            }
        }
        "#;

        let program = parse_source(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
        
        if let Declaration::Behavior(behavior) = &program.declarations[0] {
            assert_eq!(behavior.name, "ProcessData");
            assert!(behavior.input.is_some());
            assert!(behavior.output.is_some());
            assert!(!behavior.steps.statements.is_empty());
        } else {
            panic!("Expected behavior declaration");
        }
    }

    #[test]
    fn test_function_definition() {
        let source = r#"
        function add(a: number, b: number) -> number {
            return a + b
        }
        "#;

        let program = parse_source(source).unwrap();
        assert_eq!(program.declarations.len(), 1);
        
        if let Declaration::Function(func) = &program.declarations[0] {
            assert_eq!(func.name, "add");
            assert_eq!(func.parameters.parameters.len(), 2);
            assert!(func.return_type.is_some());
        } else {
            panic!("Expected function declaration");
        }
    }
}