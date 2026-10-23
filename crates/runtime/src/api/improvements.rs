use super::{api_keys::ValidatedKey, traits::RuntimeApiProvider};
use crate::improvement::administration::{Inspection, Operation};
use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

async fn apply(
    provider: Arc<dyn RuntimeApiProvider>,
    validated: Option<Extension<ValidatedKey>>,
    workflow: String,
    operation: Operation,
) -> Response {
    let mut response = match super::routes::require_admin(validated.as_deref()) {
        Err(error) => error.into_response(),
        Ok(()) => match provider.improvement_operation(workflow, operation).await {
            Ok(value) => Json(value).into_response(),
            Err(error) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error":error})),
            )
                .into_response(),
        },
    };
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

pub async fn inspect(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    validated: Option<Extension<ValidatedKey>>,
    Path(workflow): Path<String>,
    Query(selection): Query<Inspection>,
) -> Response {
    apply(
        provider,
        validated,
        workflow,
        Operation::Inspect { selection },
    )
    .await
}

pub async fn act(
    State(provider): State<Arc<dyn RuntimeApiProvider>>,
    validated: Option<Extension<ValidatedKey>>,
    Path(workflow): Path<String>,
    Json(operation): Json<Operation>,
) -> Response {
    apply(provider, validated, workflow, operation).await
}
