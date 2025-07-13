//! SchemaPin Integration Module
//! 
//! Provides integration with the SchemaPin Go CLI for schema verification

pub mod types;
pub mod cli_wrapper;
pub mod key_store;

// Re-export main types and traits for convenience
pub use types::{
    SchemaPinConfig, SchemaPinError, VerificationResult, VerifyArgs, SignatureInfo,
    SigningResult, SignArgs, PinnedKey, KeyStoreConfig, KeyStoreError
};
pub use cli_wrapper::{
    SchemaPinCli, SchemaPinCliWrapper, MockSchemaPinCli
};
pub use key_store::LocalKeyStore;