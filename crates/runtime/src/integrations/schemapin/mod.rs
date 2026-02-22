//! SchemaPin Integration Module
//!
//! Provides integration with SchemaPin for schema verification using the
//! native Rust implementation

pub mod key_store;
pub mod native_client;
pub mod types;

// Re-export main types and traits for convenience
pub use key_store::LocalKeyStore;
pub use native_client::{MockNativeSchemaPinClient, NativeSchemaPinClient, SchemaPinClient};
pub use types::{
    KeyStoreConfig, KeyStoreError, PinnedKey, SchemaPinError, SignArgs, SignatureInfo,
    SigningResult, VerificationResult, VerifyArgs,
};

/// Default SchemaPin client type - uses native implementation
pub type DefaultSchemaPinClient = NativeSchemaPinClient;
