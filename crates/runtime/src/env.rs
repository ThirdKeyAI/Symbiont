//! Strict parser for the `SYMBIONT_ENV` environment variable.
//!
//! Earlier revisions inspected `SYMBIONT_ENV` directly with loose string
//! comparisons, so `SYMBIONT_ENV=prod` or `SYMBIONT_ENV=production-like`
//! would silently bypass the production guards attached to native
//! execution, Vault TLS bypass, Swagger UI, etc. This module centralises
//! that parsing so every guard agrees on what "production" means.
//!
//! Unknown values are refused with [`EnvError::Unknown`]. Callers that need
//! production-or-else semantics should treat `Err` as "refuse to proceed".
//! Callers that merely check "is this production?" should use
//! [`is_production`] which propagates the error.

use std::fmt;

/// Parsed deployment environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Environment {
    Development,
    Staging,
    Production,
    /// Test runs (including `cargo test`) that must not be treated as
    /// production but still need a stable, non-development default.
    Test,
}

impl Environment {
    /// Return `true` when the environment is `Production`.
    pub fn is_production(&self) -> bool {
        matches!(self, Environment::Production)
    }

    /// Return `true` when the environment is the conservative default.
    /// Used for "development-only" feature gates.
    pub fn is_development(&self) -> bool {
        matches!(self, Environment::Development)
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Environment::Development => f.write_str("development"),
            Environment::Staging => f.write_str("staging"),
            Environment::Production => f.write_str("production"),
            Environment::Test => f.write_str("test"),
        }
    }
}

/// Error raised when `SYMBIONT_ENV` carries a value outside the allowed set.
#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    #[error(
        "SYMBIONT_ENV has unknown value {value:?}; \
         allowed values are: development, staging, production, test"
    )]
    Unknown { value: String },
}

/// Parse `SYMBIONT_ENV` from the process environment.
///
/// Returns `Ok(Environment::Development)` when the variable is unset.
pub fn current() -> Result<Environment, EnvError> {
    match std::env::var("SYMBIONT_ENV") {
        Ok(raw) => parse(&raw),
        Err(_) => Ok(Environment::Development),
    }
}

/// Parse a `SYMBIONT_ENV`-style string into an [`Environment`].
///
/// Accepts exact case-insensitive matches. Unknown values return
/// [`EnvError::Unknown`] so misconfigurations fail loudly rather than
/// silently dropping back to development.
pub fn parse(raw: &str) -> Result<Environment, EnvError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "development" | "dev" | "local" => Ok(Environment::Development),
        "staging" | "stage" | "preview" => Ok(Environment::Staging),
        "production" | "prod" => Ok(Environment::Production),
        "test" | "ci" => Ok(Environment::Test),
        "" => Ok(Environment::Development),
        _ => Err(EnvError::Unknown {
            value: raw.to_string(),
        }),
    }
}

/// `true` iff the current environment is production.
///
/// Returns `Err` on an unparseable `SYMBIONT_ENV`. Guards that care about
/// production-only behaviour should treat `Err` as "refuse to start" so
/// typos cannot silently downgrade the environment to dev.
pub fn is_production() -> Result<bool, EnvError> {
    current().map(|e| e.is_production())
}

/// Convenience helper for guards that want "return Err when this is
/// production". Wraps the environment error into whatever error type the
/// caller uses via [`From`].
pub fn require_non_production<E: From<EnvError> + From<String>>(reason: &str) -> Result<(), E> {
    if is_production()? {
        return Err(E::from(format!(
            "{reason} is not permitted when SYMBIONT_ENV=production"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_values_parse() {
        assert_eq!(parse("development").unwrap(), Environment::Development);
        assert_eq!(parse("staging").unwrap(), Environment::Staging);
        assert_eq!(parse("production").unwrap(), Environment::Production);
        assert_eq!(parse("test").unwrap(), Environment::Test);
    }

    #[test]
    fn common_aliases_parse() {
        assert_eq!(parse("dev").unwrap(), Environment::Development);
        assert_eq!(parse("prod").unwrap(), Environment::Production);
        assert_eq!(parse("PROD").unwrap(), Environment::Production);
        assert_eq!(parse("  production  ").unwrap(), Environment::Production);
        assert_eq!(parse("").unwrap(), Environment::Development);
    }

    #[test]
    fn unknown_value_is_refused() {
        let err = parse("production-like").expect_err("must fail");
        let msg = format!("{err}");
        assert!(msg.contains("production-like"), "{msg}");
        assert!(msg.contains("allowed values"), "{msg}");
    }

    #[test]
    fn typo_style_values_are_refused() {
        assert!(parse("prodcution").is_err());
        assert!(parse("live").is_err());
        assert!(parse("master").is_err());
    }
}
