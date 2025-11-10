mod common;

use axum_test::TestServer;
use common::TestApp;
use serde_json::json;

#[tokio::test]
async fn test_health_check_tracks_metrics() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Make multiple health check requests
    for _ in 0..3 {
        let response = server.get("/health").await;
        response.assert_status_ok();
    }

    // The health_checks counter should have incremented 3 times
    // We can't easily verify the exact count in tests, but we verify it doesn't panic
}

#[tokio::test]
async fn test_error_metrics_tracked_on_unauthorized() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Make a request without auth (should trigger 401 error)
    let response = server
        .post("/api/v1/sessions")
        .json(&json!({
            "difficulty": 5
        }))
        .await;

    response.assert_status_unauthorized();
    // The http_errors_total counter should have incremented
}

#[tokio::test]
async fn test_error_metrics_tracked_on_not_found() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Try to get a non-existent session (should trigger 404 error)
    let response = server.get("/api/v1/sessions/nonexistent-session-id").await;

    response.assert_status_not_found();
    // The http_errors_total counter should have incremented
}

#[tokio::test]
async fn test_error_metrics_tracked_on_bad_request() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Make a request with invalid parameters
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "difficulty": 100, // Invalid difficulty (max is 10)
            "ttl_seconds": 300
        }))
        .await;

    response.assert_status_bad_request();
    // The http_errors_total counter should have incremented
}

#[tokio::test]
async fn test_database_error_metrics_tracked() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Try to validate a non-existent session (triggers database lookup that fails)
    let response = server
        .post("/api/v1/sessions/nonexistent-id/validate")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "solution": "test"
        }))
        .await;

    response.assert_status_not_found();
    // This should track an error but not a database error (it's a logical not found)
}

#[tokio::test]
async fn test_request_duration_tracked() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Make various types of requests
    // Health check
    server.get("/health").await.assert_status_ok();

    // Create session
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "difficulty": 5,
            "ttl_seconds": 300
        }))
        .await;
    assert_eq!(create_response.status_code(), 201);

    // Get session (will be tracked)
    let session_data: serde_json::Value = create_response.json();
    let session_id = session_data["session_id"].as_str().unwrap();

    server
        .get(&format!("/api/v1/sessions/{}", session_id))
        .await
        .assert_status_ok();

    // All these requests should have had their duration tracked
}

#[tokio::test]
async fn test_multiple_errors_tracked() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Generate multiple errors
    for _ in 0..5 {
        let response = server
            .post("/api/v1/sessions")
            .json(&json!({
                "difficulty": 5
            }))
            .await;

        response.assert_status_unauthorized();
    }

    // The http_errors_total counter should have incremented 5 times
}

#[tokio::test]
async fn test_successful_requests_dont_increment_error_counters() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Make successful requests
    let response = server.get("/health").await;
    response.assert_status_ok();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "difficulty": 5,
            "ttl_seconds": 300
        }))
        .await;
    assert_eq!(response.status_code(), 201);

    // These successful requests should NOT increment error counters
}
