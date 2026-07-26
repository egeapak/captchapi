pub mod admin;
#[cfg(feature = "admin-ui")]
pub mod admin_ui;
pub mod api_keys;
pub mod health;
pub mod sessions;

pub use admin::admin_routes;
#[cfg(feature = "admin-ui")]
pub use admin_ui::admin_ui_routes;
pub use api_keys::api_keys_routes;
pub use health::health_check;
pub use sessions::sessions_routes;
