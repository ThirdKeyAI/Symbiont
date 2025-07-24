//! SchemaPin Integration Module
//!
//! Provides integration with SchemaPin for schema verification using both
//! native Rust implementation and legacy CLI wrapper support

pub mod cli_wrapper;
pub mod key_store;
pub mod native_client;
pub mod types;

// Re-export main types and traits for convenience
pub use cli_wrapper::{MockSchemaPinCli, SchemaPinCli, SchemaPinCliWrapper};
pub use key_store::LocalKeyStore;
pub use native_client::{MockNativeSchemaPinClient, NativeSchemaPinClient, SchemaPinClient};
pub use types::{
    KeyStoreConfig, KeyStoreError, PinnedKey, SchemaPinConfig, SchemaPinError, SignArgs,
    SignatureInfo, SigningResult, VerificationResult, VerifyArgs,
};

/// Default SchemaPin client type - uses native implementation
pub type DefaultSchemaPinClient = NativeSchemaPinClient;
