use crate::metrics::Metrics;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use std::sync::Arc;

#[tracing::instrument(skip(metrics))]
pub async fn health_check(State(metrics): State<Arc<Metrics>>) -> Json<Value> {
    // Track health check metric
    metrics.system.health_checks.add(1, &[]);

    Json(json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION")
    }))
}
