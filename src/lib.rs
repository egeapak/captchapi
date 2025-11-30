// Library exports for testing and potential library usage

pub mod config;
pub mod error;
pub mod metrics;
pub mod middleware;
pub mod models;
pub mod routes;
pub mod services;
pub mod tasks;
pub mod telemetry;

// Node.js bindings (only when napi feature is enabled)
#[cfg(feature = "napi")]
pub mod napi;
