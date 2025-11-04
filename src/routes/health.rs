use axum::Json;
use serde_json::{json, Value};

#[tracing::instrument]
pub async fn health_check() -> Json<Value> {
    Json(json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION")
    }))
}
