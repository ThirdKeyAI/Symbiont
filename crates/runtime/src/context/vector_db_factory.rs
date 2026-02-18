//! Vector backend factory.
//!
//! Resolves which vector backend to use from env vars / config,
//! then constructs and returns `Arc<dyn VectorDb>`.

use std::path::PathBuf;
use std::sync::Arc;

use crate::context::types::ContextError;
use crate::context::vector_db::NoOpVectorDatabase;
use crate::context::vector_db_lance::{LanceDbBackend, LanceDbConfig};
use crate::context::vector_db_trait::{DistanceMetric, VectorDb};

/// Backend selection config.
#[derive(Debug, Clone)]
pub enum VectorBackendConfig {
    LanceDb(LanceDbConfig),
    #[cfg(feature = "vector-qdrant")]
    Qdrant(crate::context::vector_db::QdrantConfig),
    NoOp,
}

impl Default for VectorBackendConfig {
    fn default() -> Self {
        Self::LanceDb(LanceDbConfig::default())
    }
}

/// Build the appropriate vector backend from config.
pub async fn create_vector_backend(
    config: VectorBackendConfig,
) -> Result<Arc<dyn VectorDb>, ContextError> {
    match config {
        VectorBackendConfig::LanceDb(cfg) => {
            let backend = LanceDbBackend::new(cfg).await?;
            Ok(Arc::new(backend))
        }
        #[cfg(feature = "vector-qdrant")]
        VectorBackendConfig::Qdrant(cfg) => {
            let backend = crate::context::vector_db::QdrantClientWrapper::new(cfg);
            Ok(Arc::new(backend))
        }
        VectorBackendConfig::NoOp => Ok(Arc::new(NoOpVectorDatabase)),
    }
}

/// Resolve vector backend config from environment variables.
///
/// Resolution order:
/// 1. `SYMBIONT_VECTOR_BACKEND` env var (`lancedb` | `qdrant` | `noop`)
/// 2. Default: `lancedb`
///
/// Additional env vars:
/// - `SYMBIONT_VECTOR_DATA_PATH` — LanceDB data directory (default: `./data/vector_db`)
/// - `SYMBIONT_VECTOR_HOST` — Qdrant host (default: `localhost`)
/// - `SYMBIONT_VECTOR_PORT` — Qdrant port (default: `6333`)
/// - `SYMBIONT_VECTOR_API_KEY` — Qdrant API key (optional)
/// - `SYMBIONT_VECTOR_COLLECTION` — Collection name (default: `symbiont_context`)
/// - `SYMBIONT_VECTOR_DIMENSION` — Vector dimension (default: `384`)
pub fn resolve_vector_config() -> VectorBackendConfig {
    let backend =
        std::env::var("SYMBIONT_VECTOR_BACKEND").unwrap_or_else(|_| "lancedb".to_string());

    let dimension: usize = std::env::var("SYMBIONT_VECTOR_DIMENSION")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(384);

    let collection = std::env::var("SYMBIONT_VECTOR_COLLECTION")
        .unwrap_or_else(|_| "symbiont_context".to_string());

    match backend.to_lowercase().as_str() {
        #[cfg(feature = "vector-qdrant")]
        "qdrant" => {
            let host =
                std::env::var("SYMBIONT_VECTOR_HOST").unwrap_or_else(|_| "localhost".to_string());
            let port = std::env::var("SYMBIONT_VECTOR_PORT").unwrap_or_else(|_| "6333".to_string());
            let api_key = std::env::var("SYMBIONT_VECTOR_API_KEY").ok();
            VectorBackendConfig::Qdrant(crate::context::vector_db::QdrantConfig {
                url: format!("http://{}:{}", host, port),
                api_key,
                collection_name: collection,
                vector_dimension: dimension,
                ..Default::default()
            })
        }
        "noop" | "none" => VectorBackendConfig::NoOp,
        _ => {
            let path = std::env::var("SYMBIONT_VECTOR_DATA_PATH")
                .unwrap_or_else(|_| "./data/vector_db".to_string());
            VectorBackendConfig::LanceDb(LanceDbConfig {
                data_path: PathBuf::from(path),
                collection_name: collection,
                vector_dimension: dimension,
                distance_metric: DistanceMetric::Cosine,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_defaults_to_lancedb() {
        std::env::remove_var("SYMBIONT_VECTOR_BACKEND");
        let config = resolve_vector_config();
        assert!(matches!(config, VectorBackendConfig::LanceDb(_)));
    }

    #[test]
    fn test_resolve_lancedb_explicit() {
        std::env::set_var("SYMBIONT_VECTOR_BACKEND", "lancedb");
        let config = resolve_vector_config();
        assert!(matches!(config, VectorBackendConfig::LanceDb(_)));
        std::env::remove_var("SYMBIONT_VECTOR_BACKEND");
    }

    #[test]
    fn test_resolve_custom_data_path() {
        std::env::set_var("SYMBIONT_VECTOR_BACKEND", "lancedb");
        std::env::set_var("SYMBIONT_VECTOR_DATA_PATH", "/tmp/custom_vectors");
        let config = resolve_vector_config();
        match config {
            VectorBackendConfig::LanceDb(cfg) => {
                assert_eq!(cfg.data_path, PathBuf::from("/tmp/custom_vectors"));
            }
            #[allow(unreachable_patterns)]
            _ => panic!("Expected LanceDb config"),
        }
        std::env::remove_var("SYMBIONT_VECTOR_BACKEND");
        std::env::remove_var("SYMBIONT_VECTOR_DATA_PATH");
    }

    #[tokio::test]
    async fn test_create_lance_backend() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = VectorBackendConfig::LanceDb(LanceDbConfig {
            data_path: tmp.path().to_path_buf(),
            ..Default::default()
        });
        let backend = create_vector_backend(config).await;
        assert!(backend.is_ok());
    }

    #[tokio::test]
    async fn test_create_noop_backend() {
        let backend = create_vector_backend(VectorBackendConfig::NoOp).await;
        assert!(backend.is_ok());
    }
}
