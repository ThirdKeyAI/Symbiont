use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChannelAdapterError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("authentication error: {0}")]
    Auth(String),

    #[error("connection error: {0}")]
    Connection(String),

    #[error("message send failed: {0}")]
    SendFailed(String),

    #[error("message parse error: {0}")]
    ParseError(String),

    #[error("signature verification failed: {0}")]
    SignatureInvalid(String),

    #[error("adapter not running")]
    NotRunning,

    #[error("adapter already running")]
    AlreadyRunning,

    #[error("agent invocation failed: {0}")]
    AgentError(String),

    #[error("policy denied: {0}")]
    PolicyDenied(String),

    #[error("rate limited")]
    RateLimited,

    #[error("internal error: {0}")]
    Internal(String),
}
