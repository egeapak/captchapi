mod common;

use axum_test::TestServer;
use common::TestApp;
use serde_json::json;

#[tokio::test]
async fn test_create_api_key_without_master_key_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/api-keys")
        .json(&json!({
            "description": "Test Key"
        }))
        .await;

    response.assert_status_unauthorized();
}

#[tokio::test]
async fn test_create_api_key_with_master_key_succeeds() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "description": "New Test Key"
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = response.json();

    assert!(body.get("api_key").is_some());
    assert!(body.get("key_hash").is_some());
    assert_eq!(body["description"], "New Test Key");
    assert!(body.get("created_at").is_some());

    // API key should be 32 characters
    let api_key = body["api_key"].as_str().unwrap();
    assert_eq!(api_key.len(), 32);
}

#[tokio::test]
async fn test_list_api_keys() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create a couple of API keys
    server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "description": "Key 1"
        }))
        .await;

    server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "description": "Key 2"
        }))
        .await;

    // List all keys
    let response = server
        .get("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let keys = body.as_array().unwrap();

    // Should have at least 3 keys (1 from setup + 2 created)
    assert!(keys.len() >= 3);

    // Check structure of first key
    let first_key = &keys[0];
    assert!(first_key.get("key_hash").is_some());
    assert!(first_key.get("created_at").is_some());
    assert!(first_key.get("is_active").is_some());
}

#[tokio::test]
async fn test_update_api_key() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create an API key
    let create_response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "description": "Original Description"
        }))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let key_hash = create_body["key_hash"].as_str().unwrap();

    // Update the key (deactivate it)
    let update_response = server
        .put(&format!("/api/v1/api-keys/{}", key_hash))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "is_active": false,
            "description": "Updated Description"
        }))
        .await;

    update_response.assert_status_ok();
    let update_body: serde_json::Value = update_response.json();

    assert_eq!(update_body["is_active"], false);
    assert_eq!(update_body["description"], "Updated Description");
}

#[tokio::test]
async fn test_delete_api_key() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create an API key
    let create_response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "description": "To Be Deleted"
        }))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let key_hash = create_body["key_hash"].as_str().unwrap();

    // Delete the key
    let delete_response = server
        .delete(&format!("/api/v1/api-keys/{}", key_hash))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .await;

    delete_response.assert_status(axum::http::StatusCode::NO_CONTENT);

    // Verify it's deleted by listing keys
    let list_response = server
        .get("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .await;

    let list_body: serde_json::Value = list_response.json();
    let keys = list_body.as_array().unwrap();

    // The deleted key should not be in the list
    assert!(!keys.iter().any(|k| k["key_hash"] == key_hash));
}

#[tokio::test]
async fn test_deactivated_api_key_cannot_access_sessions() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create an API key
    let create_response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "description": "Test Key"
        }))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let api_key = create_body["api_key"].as_str().unwrap();
    let key_hash = create_body["key_hash"].as_str().unwrap();

    // Verify the key works
    let session_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", api_key))
        .json(&json!({}))
        .await;

    session_response.assert_status(axum::http::StatusCode::CREATED);

    // Deactivate the key
    server
        .put(&format!("/api/v1/api-keys/{}", key_hash))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "is_active": false
        }))
        .await;

    // Try to use the deactivated key
    let session_response2 = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", api_key))
        .json(&json!({}))
        .await;

    session_response2.assert_status_unauthorized();
}

#[tokio::test]
async fn test_create_and_use_api_key_end_to_end() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create an API key using master key
    let create_response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "description": "End-to-end test key"
        }))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let new_api_key = create_body["api_key"].as_str().unwrap();

    // Use the new API key to create a session
    let session_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", new_api_key))
        .json(&json!({
            "text": "TEST",
            "difficulty": 5
        }))
        .await;

    session_response.assert_status(axum::http::StatusCode::CREATED);
    let session_body: serde_json::Value = session_response.json();
    assert!(session_body.get("session_id").is_some());
}

#[tokio::test]
async fn test_update_nonexistent_api_key() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .put("/api/v1/api-keys/nonexistent-hash")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "is_active": false
        }))
        .await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_create_api_key_with_empty_description_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({ "description": "" }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_api_key_parameters");
}

#[tokio::test]
async fn test_create_api_key_with_description_too_long_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let long_desc: String = "a".repeat(256);
    let response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({ "description": long_desc }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_api_key_parameters");
}

#[tokio::test]
async fn test_update_api_key_with_empty_description_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create a key first
    let create_response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({ "description": "Valid key" }))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let key_hash = create_body["key_hash"].as_str().unwrap();

    // Try to update with empty description
    let response = server
        .put(&format!("/api/v1/api-keys/{}", key_hash))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({ "description": "" }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_api_key_parameters");
}

#[tokio::test]
async fn test_update_api_key_with_description_too_long_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create a key first
    let create_response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({ "description": "Valid key" }))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let key_hash = create_body["key_hash"].as_str().unwrap();

    // Try to update with too-long description
    let long_desc: String = "a".repeat(256);
    let response = server
        .put(&format!("/api/v1/api-keys/{}", key_hash))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({ "description": long_desc }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_api_key_parameters");
}

#[tokio::test]
async fn test_create_api_key_with_whitespace_only_description_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({ "description": "   " }))
        .await;

    response.assert_status_bad_request();
}

#[tokio::test]
async fn test_update_api_key_with_whitespace_only_description_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create an API key first
    let create_response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({ "description": "Test Key" }))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let key_hash = create_body["key_hash"].as_str().unwrap();

    // Try to update with whitespace-only description
    let response = server
        .put(&format!("/api/v1/api-keys/{}", key_hash))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({ "description": "   " }))
        .await;

    response.assert_status_bad_request();
}

#[tokio::test]
async fn test_create_api_key_with_control_chars_description_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({ "description": "test\x00key" }))
        .await;

    response.assert_status_bad_request();
}
