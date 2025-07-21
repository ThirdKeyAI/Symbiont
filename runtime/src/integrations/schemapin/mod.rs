//! SchemaPin Integration Module
//!
//! Provides integration with SchemaPin for schema verification using both
//! native Rust implementation and legacy CLI wrapper support

pub mod types;
pub mod cli_wrapper;
pub mod native_client;
pub mod key_store;

// Re-export main types and traits for convenience
pub use types::{
    SchemaPinConfig, SchemaPinError, VerificationResult, VerifyArgs, SignatureInfo,
    SigningResult, SignArgs, PinnedKey, KeyStoreConfig, KeyStoreError
};
pub use cli_wrapper::{
    SchemaPinCli, SchemaPinCliWrapper, MockSchemaPinCli
};
pub use native_client::{
    SchemaPinClient, NativeSchemaPinClient, MockNativeSchemaPinClient
};
pub use key_store::LocalKeyStore;

/// Default SchemaPin client type - uses native implementation
pub type DefaultSchemaPinClient = NativeSchemaPinClient;