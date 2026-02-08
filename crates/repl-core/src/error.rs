use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum ReplError {
    #[error("Evaluation failed: {0}")]
    Evaluation(String),
    #[error("Policy parsing failed: {0}")]
    PolicyParsing(String),
    #[error("Lexing failed: {0}")]
    Lexing(String),
    #[error("Parsing failed: {0}")]
    Parsing(String),
    #[error("Execution failed: {0}")]
    Execution(String),
    #[error("Security error: {0}")]
    Security(String),
    #[error("Runtime error: {0}")]
    Runtime(String),
    #[error("IO error: {0}")]
    Io(String),
    #[error("JSON error: {0}")]
    Json(String),
    #[error("UUID error: {0}")]
    Uuid(String),
}

impl From<std::io::Error> for ReplError {
    fn from(error: std::io::Error) -> Self {
        ReplError::Io(error.to_string())
    }
}

impl From<serde_json::Error> for ReplError {
    fn from(error: serde_json::Error) -> Self {
        ReplError::Json(error.to_string())
    }
}

impl From<uuid::Error> for ReplError {
    fn from(error: uuid::Error) -> Self {
        ReplError::Uuid(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ReplError>;
