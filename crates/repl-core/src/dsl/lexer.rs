//! Lexer for the Symbiont DSL
//!
//! Converts raw text input into a stream of tokens for parsing.

use crate::error::{ReplError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Token types recognized by the lexer
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TokenType {
    // Literals
    String(String),
    Number(f64),
    Integer(i64),
    Boolean(bool),
    Duration(u64, String), // value, unit
    Size(u64, String),     // value, unit
    
    // Identifiers and keywords
    Identifier(String),
    Keyword(Keyword),
    
    // Operators
    Plus,
    Minus,
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
    Not,
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    BitwiseNot,
    LeftShift,
    RightShift,
    Assign,
    Question,
    
    // Delimiters
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Semicolon,
    Colon,
    Dot,
    Arrow,
    FatArrow,
    
    // Special
    Newline,
    Eof,
    Comment(String),
}

/// Keywords in the DSL
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Keyword {
    Agent,
    Behavior,
    Function,
    Struct,
    Let,
    If,
    Else,
    Match,
    For,
    While,
    Try,
    Catch,
    Return,
    Emit,
    Require,
    Check,
    On,
    In,
    Invoke,
    True,
    False,
    Null,
    Capability,
    Capabilities,
    Policy,
    Has,
    Name,
    Version,
    Author,
    Description,
    Resources,
    Security,
    Policies,
    Input,
    Output,
    Steps,
    Memory,
    Cpu,
    Network,
    Storage,
    Tier,
    Sandbox,
    Allow,
    Strict,
    Moderate,
    Permissive,
    Timeout,
    Retry,
    Failure,
    Terminate,
    Restart,
    Escalate,
    Ignore,
    Tier1,
    Tier2,
    Tier3,
    Tier4,
}

/// Token with location information
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub token_type: TokenType,
    pub line: usize,
    pub column: usize,
    pub offset: usize,
    pub length: usize,
}

/// Lexer for the Symbiont DSL
pub struct Lexer {
    input: Vec<char>,
    position: usize,
    line: usize,
    column: usize,
    keywords: HashMap<String, Keyword>,
}

impl Lexer {
    /// Create a new lexer for the given input
    pub fn new(input: &str) -> Self {
        let mut keywords = HashMap::new();
        
        // Insert all keywords
        keywords.insert("agent".to_string(), Keyword::Agent);
        keywords.insert("behavior".to_string(), Keyword::Behavior);
        keywords.insert("function".to_string(), Keyword::Function);
        keywords.insert("struct".to_string(), Keyword::Struct);
        keywords.insert("let".to_string(), Keyword::Let);
        keywords.insert("if".to_string(), Keyword::If);
        keywords.insert("else".to_string(), Keyword::Else);
        keywords.insert("match".to_string(), Keyword::Match);
        keywords.insert("for".to_string(), Keyword::For);
        keywords.insert("while".to_string(), Keyword::While);
        keywords.insert("try".to_string(), Keyword::Try);
        keywords.insert("catch".to_string(), Keyword::Catch);
        keywords.insert("return".to_string(), Keyword::Return);
        keywords.insert("emit".to_string(), Keyword::Emit);
        keywords.insert("require".to_string(), Keyword::Require);
        keywords.insert("check".to_string(), Keyword::Check);
        keywords.insert("on".to_string(), Keyword::On);
        keywords.insert("in".to_string(), Keyword::In);
        keywords.insert("invoke".to_string(), Keyword::Invoke);
        keywords.insert("true".to_string(), Keyword::True);
        keywords.insert("false".to_string(), Keyword::False);
        keywords.insert("null".to_string(), Keyword::Null);
        keywords.insert("capability".to_string(), Keyword::Capability);
        keywords.insert("capabilities".to_string(), Keyword::Capabilities);
        keywords.insert("policy".to_string(), Keyword::Policy);
        keywords.insert("has".to_string(), Keyword::Has);
        keywords.insert("name".to_string(), Keyword::Name);
        keywords.insert("version".to_string(), Keyword::Version);
        keywords.insert("author".to_string(), Keyword::Author);
        keywords.insert("description".to_string(), Keyword::Description);
        keywords.insert("resources".to_string(), Keyword::Resources);
        keywords.insert("security".to_string(), Keyword::Security);
        keywords.insert("policies".to_string(), Keyword::Policies);
        keywords.insert("input".to_string(), Keyword::Input);
        keywords.insert("output".to_string(), Keyword::Output);
        keywords.insert("steps".to_string(), Keyword::Steps);
        keywords.insert("memory".to_string(), Keyword::Memory);
        keywords.insert("cpu".to_string(), Keyword::Cpu);
        keywords.insert("network".to_string(), Keyword::Network);
        keywords.insert("storage".to_string(), Keyword::Storage);
        keywords.insert("tier".to_string(), Keyword::Tier);
        keywords.insert("sandbox".to_string(), Keyword::Sandbox);
        keywords.insert("allow".to_string(), Keyword::Allow);
        keywords.insert("strict".to_string(), Keyword::Strict);
        keywords.insert("moderate".to_string(), Keyword::Moderate);
        keywords.insert("permissive".to_string(), Keyword::Permissive);
        keywords.insert("timeout".to_string(), Keyword::Timeout);
        keywords.insert("retry".to_string(), Keyword::Retry);
        keywords.insert("failure".to_string(), Keyword::Failure);
        keywords.insert("terminate".to_string(), Keyword::Terminate);
        keywords.insert("restart".to_string(), Keyword::Restart);
        keywords.insert("escalate".to_string(), Keyword::Escalate);
        keywords.insert("ignore".to_string(), Keyword::Ignore);
        keywords.insert("Tier1".to_string(), Keyword::Tier1);
        keywords.insert("Tier2".to_string(), Keyword::Tier2);
        keywords.insert("Tier3".to_string(), Keyword::Tier3);
        keywords.insert("Tier4".to_string(), Keyword::Tier4);

        Self {
            input: input.chars().collect(),
            position: 0,
            line: 1,
            column: 1,
            keywords,
        }
    }

    /// Tokenize the entire input
    pub fn tokenize(&mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        
        loop {
            let token = self.next_token()?;
            let is_eof = matches!(token.token_type, TokenType::Eof);
            tokens.push(token);
            
            if is_eof {
                break;
            }
        }
        
        Ok(tokens)
    }

    /// Get the next token
    pub fn next_token(&mut self) -> Result<Token> {
        self.skip_whitespace();
        
        let start_line = self.line;
        let start_column = self.column;
        let start_offset = self.position;
        
        if self.position >= self.input.len() {
            return Ok(Token {
                token_type: TokenType::Eof,
                line: start_line,
                column: start_column,
                offset: start_offset,
                length: 0,
            });
        }
        
        let ch = self.current_char();
        
        let token_type = match ch {
            // Comments
            '/' if self.peek_char() == Some('/') => {
                let comment = self.read_line_comment();
                TokenType::Comment(comment)
            }
            '/' if self.peek_char() == Some('*') => {
                let comment = self.read_block_comment()?;
                TokenType::Comment(comment)
            }
            
            // String literals
            '"' => {
                let string = self.read_string()?;
                TokenType::String(string)
            }
            
            // Numbers
            c if c.is_ascii_digit() => {
                self.read_number()?
            }
            
            // Identifiers and keywords
            c if c.is_alphabetic() || c == '_' => {
                let identifier = self.read_identifier();
                if let Some(keyword) = self.keywords.get(&identifier) {
                    TokenType::Keyword(keyword.clone())
                } else {
                    TokenType::Identifier(identifier)
                }
            }
            
            // Operators and punctuation
            '+' => { self.advance(); TokenType::Plus }
            '-' if self.peek_char() == Some('>') => {
                self.advance(); // -
                self.advance(); // >
                TokenType::Arrow
            }
            '-' => { self.advance(); TokenType::Minus }
            '*' => { self.advance(); TokenType::Multiply }
            '/' => { self.advance(); TokenType::Divide }
            '%' => { self.advance(); TokenType::Modulo }
            '=' if self.peek_char() == Some('=') => {
                self.advance(); // =
                self.advance(); // =
                TokenType::Equal
            }
            '=' if self.peek_char() == Some('>') => {
                self.advance(); // =
                self.advance(); // >
                TokenType::FatArrow
            }
            '=' => { self.advance(); TokenType::Assign }
            '!' if self.peek_char() == Some('=') => {
                self.advance(); // !
                self.advance(); // =
                TokenType::NotEqual
            }
            '!' => { self.advance(); TokenType::Not }
            '<' if self.peek_char() == Some('=') => {
                self.advance(); // <
                self.advance(); // =
                TokenType::LessThanOrEqual
            }
            '<' if self.peek_char() == Some('<') => {
                self.advance(); // <
                self.advance(); // <
                TokenType::LeftShift
            }
            '<' => { self.advance(); TokenType::LessThan }
            '>' if self.peek_char() == Some('=') => {
                self.advance(); // >
                self.advance(); // =
                TokenType::GreaterThanOrEqual
            }
            '>' if self.peek_char() == Some('>') => {
                self.advance(); // >
                self.advance(); // >
                TokenType::RightShift
            }
            '>' => { self.advance(); TokenType::GreaterThan }
            '&' if self.peek_char() == Some('&') => {
                self.advance(); // &
                self.advance(); // &
                TokenType::And
            }
            '&' => { self.advance(); TokenType::BitwiseAnd }
            '|' if self.peek_char() == Some('|') => {
                self.advance(); // |
                self.advance(); // |
                TokenType::Or
            }
            '|' => { self.advance(); TokenType::BitwiseOr }
            '^' => { self.advance(); TokenType::BitwiseXor }
            '~' => { self.advance(); TokenType::BitwiseNot }
            '?' => { self.advance(); TokenType::Question }
            
            // Delimiters
            '(' => { self.advance(); TokenType::LeftParen }
            ')' => { self.advance(); TokenType::RightParen }
            '{' => { self.advance(); TokenType::LeftBrace }
            '}' => { self.advance(); TokenType::RightBrace }
            '[' => { self.advance(); TokenType::LeftBracket }
            ']' => { self.advance(); TokenType::RightBracket }
            ',' => { self.advance(); TokenType::Comma }
            ';' => { self.advance(); TokenType::Semicolon }
            ':' => { self.advance(); TokenType::Colon }
            '.' => { self.advance(); TokenType::Dot }
            
            // Newlines
            '\n' => { 
                self.advance();
                self.line += 1;
                self.column = 1;
                TokenType::Newline
            }
            
            // Unexpected character
            _ => {
                return Err(ReplError::Lexing(format!(
                    "Unexpected character '{}' at line {}, column {}",
                    ch, self.line, self.column
                )));
            }
        };
        
        let length = self.position - start_offset;
        
        Ok(Token {
            token_type,
            line: start_line,
            column: start_column,
            offset: start_offset,
            length,
        })
    }

    /// Skip whitespace characters
    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char_opt() {
            if ch.is_whitespace() && ch != '\n' {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Read a string literal
    fn read_string(&mut self) -> Result<String> {
        self.advance(); // Skip opening quote
        let mut string = String::new();
        
        while let Some(ch) = self.current_char_opt() {
            match ch {
                '"' => {
                    self.advance(); // Skip closing quote
                    return Ok(string);
                }
                '\\' => {
                    self.advance(); // Skip backslash
                    if let Some(escaped) = self.current_char_opt() {
                        match escaped {
                            'n' => string.push('\n'),
                            't' => string.push('\t'),
                            'r' => string.push('\r'),
                            '\\' => string.push('\\'),
                            '"' => string.push('"'),
                            _ => {
                                string.push('\\');
                                string.push(escaped);
                            }
                        }
                        self.advance();
                    } else {
                        return Err(ReplError::Lexing("Unterminated string literal".to_string()));
                    }
                }
                '\n' => {
                    self.line += 1;
                    self.column = 1;
                    string.push(ch);
                    self.advance();
                }
                _ => {
                    string.push(ch);
                    self.advance();
                }
            }
        }
        
        Err(ReplError::Lexing("Unterminated string literal".to_string()))
    }

    /// Read a number (integer or float) with optional units
    fn read_number(&mut self) -> Result<TokenType> {
        let mut number_str = String::new();
        let mut has_dot = false;
        
        // Read digits and optional decimal point
        while let Some(ch) = self.current_char_opt() {
            if ch.is_ascii_digit() {
                number_str.push(ch);
                self.advance();
            } else if ch == '.' && !has_dot {
                has_dot = true;
                number_str.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        // Check for units (duration or size)
        if let Some(ch) = self.current_char_opt() {
            if ch.is_alphabetic() {
                let unit = self.read_unit();
                let value = if has_dot {
                    number_str.parse::<f64>()
                        .map_err(|_| ReplError::Lexing(format!("Invalid number: {}", number_str)))?
                        as u64
                } else {
                    number_str.parse::<u64>()
                        .map_err(|_| ReplError::Lexing(format!("Invalid number: {}", number_str)))?
                };
                
                // Determine if it's a duration or size unit
                if matches!(unit.as_str(), "s" | "m" | "h" | "d" | "ms") {
                    return Ok(TokenType::Duration(value, unit));
                } else if matches!(unit.as_str(), "B" | "KB" | "MB" | "GB" | "TB") {
                    return Ok(TokenType::Size(value, unit));
                }
            }
        }
        
        // Parse as regular number
        if has_dot {
            let value = number_str.parse::<f64>()
                .map_err(|_| ReplError::Lexing(format!("Invalid number: {}", number_str)))?;
            Ok(TokenType::Number(value))
        } else {
            let value = number_str.parse::<i64>()
                .map_err(|_| ReplError::Lexing(format!("Invalid number: {}", number_str)))?;
            Ok(TokenType::Integer(value))
        }
    }

    /// Read a unit suffix
    fn read_unit(&mut self) -> String {
        let mut unit = String::new();
        while let Some(ch) = self.current_char_opt() {
            if ch.is_alphabetic() {
                unit.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        unit
    }

    /// Read an identifier
    fn read_identifier(&mut self) -> String {
        let mut identifier = String::new();
        
        while let Some(ch) = self.current_char_opt() {
            if ch.is_alphanumeric() || ch == '_' {
                identifier.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        identifier
    }

    /// Read a line comment
    fn read_line_comment(&mut self) -> String {
        self.advance(); // /
        self.advance(); // /
        
        let mut comment = String::new();
        while let Some(ch) = self.current_char_opt() {
            if ch == '\n' {
                break;
            }
            comment.push(ch);
            self.advance();
        }
        
        comment
    }

    /// Read a block comment
    fn read_block_comment(&mut self) -> Result<String> {
        self.advance(); // /
        self.advance(); // *
        
        let mut comment = String::new();
        
        while self.position < self.input.len() - 1 {
            let ch = self.current_char();
            let next_ch = self.peek_char();
            
            if ch == '*' && next_ch == Some('/') {
                self.advance(); // *
                self.advance(); // /
                return Ok(comment);
            }
            
            if ch == '\n' {
                self.line += 1;
                self.column = 1;
            }
            
            comment.push(ch);
            self.advance();
        }
        
        Err(ReplError::Lexing("Unterminated block comment".to_string()))
    }

    /// Get the current character
    fn current_char(&self) -> char {
        self.input[self.position]
    }

    /// Get the current character as an option
    fn current_char_opt(&self) -> Option<char> {
        self.input.get(self.position).copied()
    }

    /// Peek at the next character
    fn peek_char(&self) -> Option<char> {
        self.input.get(self.position + 1).copied()
    }

    /// Advance to the next character
    fn advance(&mut self) {
        if self.position < self.input.len() {
            self.position += 1;
            self.column += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokens() {
        let mut lexer = Lexer::new("let x = 42");
        let tokens = lexer.tokenize().unwrap();
        
        assert_eq!(tokens.len(), 5); // let, x, =, 42, EOF
        assert!(matches!(tokens[0].token_type, TokenType::Keyword(Keyword::Let)));
        assert!(matches!(tokens[1].token_type, TokenType::Identifier(_)));
        assert!(matches!(tokens[2].token_type, TokenType::Assign));
        assert!(matches!(tokens[3].token_type, TokenType::Integer(42)));
        assert!(matches!(tokens[4].token_type, TokenType::Eof));
    }

    #[test]
    fn test_string_literal() {
        let mut lexer = Lexer::new(r#""Hello, world!""#);
        let tokens = lexer.tokenize().unwrap();
        
        assert_eq!(tokens.len(), 2); // string, EOF
        assert!(matches!(tokens[0].token_type, TokenType::String(ref s) if s == "Hello, world!"));
    }

    #[test]
    fn test_duration_literal() {
        let mut lexer = Lexer::new("30s 5m 2h");
        let tokens = lexer.tokenize().unwrap();
        
        assert_eq!(tokens.len(), 4); // 30s, 5m, 2h, EOF
        assert!(matches!(tokens[0].token_type, TokenType::Duration(30, ref unit) if unit == "s"));
        assert!(matches!(tokens[1].token_type, TokenType::Duration(5, ref unit) if unit == "m"));
        assert!(matches!(tokens[2].token_type, TokenType::Duration(2, ref unit) if unit == "h"));
    }

    #[test]
    fn test_size_literal() {
        let mut lexer = Lexer::new("1KB 512MB 2GB");
        let tokens = lexer.tokenize().unwrap();
        
        assert_eq!(tokens.len(), 4); // 1KB, 512MB, 2GB, EOF
        assert!(matches!(tokens[0].token_type, TokenType::Size(1, ref unit) if unit == "KB"));
        assert!(matches!(tokens[1].token_type, TokenType::Size(512, ref unit) if unit == "MB"));
        assert!(matches!(tokens[2].token_type, TokenType::Size(2, ref unit) if unit == "GB"));
    }

    #[test]
    fn test_comments() {
        let mut lexer = Lexer::new("// line comment\n/* block comment */");
        let tokens = lexer.tokenize().unwrap();
        
        assert_eq!(tokens.len(), 4); // line comment, newline, block comment, EOF
        assert!(matches!(tokens[0].token_type, TokenType::Comment(_)));
        assert!(matches!(tokens[1].token_type, TokenType::Newline));
        assert!(matches!(tokens[2].token_type, TokenType::Comment(_)));
    }
}