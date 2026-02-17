//! Composio integration error types

use thiserror::Error;

/// Errors that can occur during Composio MCP integration operations
#[derive(Error, Debug)]
pub enum ComposioError {
    #[error("Configuration error: {reason}")]
    ConfigError { reason: String },

    #[error("SSE transport error: {reason}")]
    TransportError { reason: String },

    #[error("Tool discovery failed for server '{server}': {reason}")]
    DiscoveryError { server: String, reason: String },

    #[error("JSON-RPC error (code {code}): {message}")]
    JsonRpcError { code: i64, message: String },

    #[error("Connection timeout to server '{server}'")]
    Timeout { server: String },

    #[error("IO error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    #[error("HTTP error: {source}")]
    Http {
        #[from]
        source: reqwest::Error,
    },

    #[error("JSON serialization error: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_error_display() {
        let err = ComposioError::ConfigError {
            reason: "missing API key".to_string(),
        };
        assert_eq!(err.to_string(), "Configuration error: missing API key");
    }

    #[test]
    fn test_transport_error_display() {
        let err = ComposioError::TransportError {
            reason: "connection refused".to_string(),
        };
        assert_eq!(err.to_string(), "SSE transport error: connection refused");
    }

    #[test]
    fn test_discovery_error_display() {
        let err = ComposioError::DiscoveryError {
            server: "github".to_string(),
            reason: "not found".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Tool discovery failed for server 'github': not found"
        );
    }

    #[test]
    fn test_jsonrpc_error_display() {
        let err = ComposioError::JsonRpcError {
            code: -32600,
            message: "Invalid Request".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "JSON-RPC error (code -32600): Invalid Request"
        );
    }

    #[test]
    fn test_timeout_error_display() {
        let err = ComposioError::Timeout {
            server: "slack".to_string(),
        };
        assert_eq!(err.to_string(), "Connection timeout to server 'slack'");
    }
}
