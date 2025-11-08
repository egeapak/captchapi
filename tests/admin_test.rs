mod common;

use axum::http::StatusCode;
use axum_test::TestServer;
use common::TestApp;
use serde_json::json;

#[tokio::test]
async fn test_cleanup_endpoint_without_master_key_fails() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app()).unwrap();

    let response = server.post("/api/v1/admin/cleanup").await;

    response.assert_status_unauthorized();
    response.assert_json(&json!({
        "error": "unauthorized",
        "message": "Missing authorization header"
    }));
}

#[tokio::test]
async fn test_cleanup_endpoint_with_invalid_master_key_fails() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app()).unwrap();

    let response = server
        .post("/api/v1/admin/cleanup")
        .add_header("Authorization", "Bearer invalid-key")
        .await;

    response.assert_status_unauthorized();
    response.assert_json(&json!({
        "error": "unauthorized",
        "message": "Invalid master key"
    }));
}

#[tokio::test]
async fn test_cleanup_endpoint_with_valid_master_key_succeeds() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app()).unwrap();

    let response = server
        .post("/api/v1/admin/cleanup")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    response.assert_status_ok();

    let body: serde_json::Value = response.json();
    assert!(body["sessions_deleted"].is_u64());
    assert!(body["message"].is_string());
}

#[tokio::test]
async fn test_cleanup_endpoint_deletes_expired_sessions() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app()).unwrap();

    // Create a session
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", app.api_key))
        .json(&json!({
            "expires_in_seconds": 1
        }))
        .await;

    create_response.assert_status(StatusCode::CREATED);
    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Wait for session to expire
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Trigger cleanup
    let cleanup_response = server
        .post("/api/v1/admin/cleanup")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    cleanup_response.assert_status_ok();
    let cleanup_body: serde_json::Value = cleanup_response.json();
    assert_eq!(cleanup_body["sessions_deleted"], 1);
    assert!(cleanup_body["message"]
        .as_str()
        .unwrap()
        .contains("Successfully cleaned up 1 expired session"));

    // Verify session is gone
    let get_response = server
        .get(&format!("/api/v1/sessions/{}", session_id))
        .await;

    get_response.assert_status_not_found();
}

#[tokio::test]
async fn test_cleanup_endpoint_with_no_expired_sessions() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app()).unwrap();

    // Create a session with long TTL
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", app.api_key))
        .json(&json!({
            "expires_in_seconds": 3600
        }))
        .await;

    create_response.assert_status(StatusCode::CREATED);

    // Trigger cleanup
    let cleanup_response = server
        .post("/api/v1/admin/cleanup")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    cleanup_response.assert_status_ok();
    let cleanup_body: serde_json::Value = cleanup_response.json();
    assert_eq!(cleanup_body["sessions_deleted"], 0);
    assert_eq!(
        cleanup_body["message"].as_str().unwrap(),
        "No expired sessions found to clean up"
    );
}

#[tokio::test]
async fn test_cleanup_endpoint_deletes_multiple_expired_sessions() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app()).unwrap();

    // Create multiple sessions with short TTL
    for _ in 0..3 {
        let create_response = server
            .post("/api/v1/sessions")
            .add_header("Authorization", format!("Bearer {}", app.api_key))
            .json(&json!({
                "expires_in_seconds": 1
            }))
            .await;

        create_response.assert_status(StatusCode::CREATED);
    }

    // Wait for sessions to expire
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Trigger cleanup
    let cleanup_response = server
        .post("/api/v1/admin/cleanup")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    cleanup_response.assert_status_ok();
    let cleanup_body: serde_json::Value = cleanup_response.json();
    assert_eq!(cleanup_body["sessions_deleted"], 3);
    assert!(cleanup_body["message"]
        .as_str()
        .unwrap()
        .contains("Successfully cleaned up 3 expired session"));
}

#[tokio::test]
async fn test_cleanup_endpoint_preserves_valid_sessions() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app()).unwrap();

    // Create expired session
    let expired_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", app.api_key))
        .json(&json!({
            "expires_in_seconds": 1
        }))
        .await;
    expired_response.assert_status(StatusCode::CREATED);

    // Create valid session
    let valid_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", app.api_key))
        .json(&json!({
            "expires_in_seconds": 3600
        }))
        .await;
    valid_response.assert_status(StatusCode::CREATED);
    let valid_body: serde_json::Value = valid_response.json();
    let valid_session_id = valid_body["session_id"].as_str().unwrap();

    // Wait for expired session to expire
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Trigger cleanup
    let cleanup_response = server
        .post("/api/v1/admin/cleanup")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    cleanup_response.assert_status_ok();
    let cleanup_body: serde_json::Value = cleanup_response.json();
    assert_eq!(cleanup_body["sessions_deleted"], 1);

    // Verify valid session still exists
    let get_response = server
        .get(&format!("/api/v1/sessions/{}", valid_session_id))
        .await;

    get_response.assert_status_ok();
}
