mod common;

use axum::http::StatusCode;
use axum_test::TestServer;
use common::TestApp;
use serde_json::json;

// ── GET /api/v1/admin/config ─────────────────────────────────────────────────

#[tokio::test]
async fn test_get_config_without_master_key_fails() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server.get("/api/v1/admin/config").await;

    response.assert_status_unauthorized();
    response.assert_json(&json!({
        "error": "unauthorized",
        "message": "Missing authorization header"
    }));
}

#[tokio::test]
async fn test_get_config_with_invalid_master_key_fails() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", "Bearer invalid-key")
        .await;

    response.assert_status_unauthorized();
}

#[tokio::test]
async fn test_get_config_returns_values_and_reloadability() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();

    assert_eq!(body["config"]["captcha_compression"]["value"], "40");
    assert_eq!(body["config"]["captcha_compression"]["reloadable"], true);
    // The listener is already bound, so the port can never be reloadable.
    assert_eq!(body["config"]["server_port"]["reloadable"], false);
    assert!(body["overrides"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_get_config_never_discloses_secrets() {
    // The master key holder can already act as an admin, but this endpoint exists to explain
    // the server's behaviour — not to read credentials back out of it.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app());

    let response = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", master_key))
        .await;

    response.assert_status_ok();
    let raw = response.text();

    assert!(
        !raw.contains("test-salt-minimum-16chars"),
        "salt leaked: {raw}"
    );
    assert!(!raw.contains(&master_key), "master key leaked: {raw}");

    let body: serde_json::Value = response.json();
    assert!(body["config"]["api_key_salt"]["value"]
        .as_str()
        .unwrap()
        .contains("redacted"));
    assert_eq!(body["config"]["api_key_salt"]["secret"], true);
    assert_eq!(body["config"]["master_api_key"]["secret"], true);
}

// ── PATCH /api/v1/admin/config ───────────────────────────────────────────────

#[tokio::test]
async fn test_patch_config_without_master_key_fails() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .patch("/api/v1/admin/config")
        .json(&json!({ "captcha_compression": 90 }))
        .await;

    response.assert_status_unauthorized();
}

#[tokio::test]
async fn test_patch_config_updates_a_reloadable_field() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "captcha_compression": 90 }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["config"]["captcha_compression"]["value"], "90");
    assert_eq!(body["overrides"], json!(["CAPTCHA_COMPRESSION"]));
}

#[tokio::test]
async fn test_patch_config_accepts_string_values() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "max_validation_attempts": "7" }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["config"]["max_validation_attempts"]["value"], "7");
}

#[tokio::test]
async fn test_patch_config_reaches_the_request_path() {
    // The test that actually proves reload works: change the default TTL, then confirm a
    // freshly created session honours it. Without this, everything else is bookkeeping.
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let before = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", app.api_key))
        .json(&json!({}))
        .await;
    before.assert_status(StatusCode::CREATED);
    let before_body: serde_json::Value = before.json();
    let before_expiry = before_body["expires_at"].as_str().unwrap().to_string();

    server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "default_session_ttl_seconds": 3000 }))
        .await
        .assert_status_ok();

    let after = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", app.api_key))
        .json(&json!({}))
        .await;
    after.assert_status(StatusCode::CREATED);
    let after_body: serde_json::Value = after.json();
    let after_expiry = after_body["expires_at"].as_str().unwrap();

    assert!(
        after_expiry > before_expiry.as_str(),
        "a longer default TTL should push the expiry out: {before_expiry} -> {after_expiry}"
    );
}

#[tokio::test]
async fn test_patch_config_enforces_the_new_attempt_limit() {
    // Lower the attempt limit to 1, then confirm the very next wrong answer exhausts it.
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "max_validation_attempts": 1 }))
        .await
        .assert_status_ok();

    let created = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", app.api_key))
        .json(&json!({}))
        .await;
    created.assert_status(StatusCode::CREATED);
    let body: serde_json::Value = created.json();
    let session_id = body["session_id"].as_str().unwrap();

    // First wrong answer consumes the only permitted attempt.
    server
        .post(&format!("/api/v1/sessions/{session_id}/validate"))
        .add_header("Authorization", format!("Bearer {}", app.api_key))
        .json(&json!({ "solution": "wrong" }))
        .await
        .assert_status_ok();

    // The second attempt trips the limit and destroys the session. Under the default limit of
    // three this attempt would simply have been counted, so the session surviving here would
    // mean the patched value never reached the request path.
    server
        .post(&format!("/api/v1/sessions/{session_id}/validate"))
        .add_header("Authorization", format!("Bearer {}", app.api_key))
        .json(&json!({ "solution": "wrong-again" }))
        .await
        .assert_status_ok();

    server
        .get(&format!("/api/v1/sessions/{session_id}"))
        .await
        .assert_status_not_found();
}

#[tokio::test]
async fn test_patch_config_rejects_a_boot_only_field() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "server_port": 9999 }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "config_not_reloadable");
    assert!(body["message"].as_str().unwrap().contains("server_port"));
}

#[tokio::test]
async fn test_patch_config_rejects_a_secret() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "master_api_key": "a-brand-new-master-key" }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "config_not_reloadable");
}

#[tokio::test]
async fn test_patch_config_rejects_an_unknown_field() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "not_a_setting": 1 }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_config");
}

#[tokio::test]
async fn test_patch_config_rejects_an_invalid_value_and_keeps_the_old_one() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "max_validation_attempts": "not-a-number" }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_config");

    // The running configuration must be untouched by a rejected patch.
    let current = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;
    let current_body: serde_json::Value = current.json();
    assert_eq!(
        current_body["config"]["max_validation_attempts"]["value"],
        "3"
    );
    assert!(current_body["overrides"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_patch_config_rejects_an_empty_body() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({}))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_config");
}

// ── POST /api/v1/admin/config/reload ─────────────────────────────────────────

#[tokio::test]
async fn test_reload_without_master_key_fails() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server.post("/api/v1/admin/config/reload").await;
    response.assert_status_unauthorized();
}

#[tokio::test]
async fn test_reload_succeeds() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .post("/api/v1/admin/config/reload")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert!(body["message"].as_str().unwrap().contains("reloaded"));
    assert!(body["ignored"].as_array().unwrap().is_empty());
    assert!(body["config"]["server_port"]["value"].is_string());
}

#[tokio::test]
async fn test_reload_discards_runtime_overrides() {
    // A reload means "re-read the sources of truth", so anything set through the API goes.
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "captcha_compression": 90 }))
        .await
        .assert_status_ok();

    let reloaded = server
        .post("/api/v1/admin/config/reload")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    reloaded.assert_status_ok();
    let body: serde_json::Value = reloaded.json();
    assert_eq!(body["config"]["captcha_compression"]["value"], "40");
    assert!(body["overrides"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_reload_preserves_boot_only_values() {
    // Reload must not silently reset the master key to some resolved default — that would
    // lock the operator out of the very endpoint they just called.
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    server
        .post("/api/v1/admin/config/reload")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await
        .assert_status_ok();

    server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await
        .assert_status_ok();
}
