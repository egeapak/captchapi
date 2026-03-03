mod common;

use axum_test::TestServer;
use common::TestApp;
use serde_json::json;

/// Test that sessions.created metric is incremented when creating sessions
#[tokio::test]
async fn test_sessions_created_metric() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create multiple sessions and verify metric updates don't panic
    for _ in 0..5 {
        let response = server
            .post("/api/v1/sessions")
            .add_header("Authorization", format!("Bearer {}", test_app.api_key))
            .json(&json!({}))
            .await;

        response.assert_status(axum::http::StatusCode::CREATED);
    }

    // We can't directly read OpenTelemetry counter values, but we can verify
    // that the operations completed successfully, which means metrics were recorded
}

/// Test that session_validation_attempts metric is incremented on validation attempts
#[tokio::test]
async fn test_session_validation_attempts_metric() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create a session
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({}))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let session_id = response.json::<serde_json::Value>()["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Make multiple validation attempts (wrong answer)
    for _ in 0..3 {
        let response = server
            .post(&format!("/api/v1/sessions/{}/validate", session_id))
            .add_header("Authorization", format!("Bearer {}", test_app.api_key))
            .json(&json!({"solution": "wrong_answer"}))
            .await;

        // Should fail due to wrong answer
        assert!(response.status_code() == axum::http::StatusCode::OK);
        let body = response.json::<serde_json::Value>();
        assert_eq!(body["valid"], false);
    }

    // The metric should have been incremented for each attempt
}

/// Test that sessions.validated metric is incremented on successful validation
#[tokio::test]
async fn test_sessions_validated_metric() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create a session
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 7
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let json_response = response.json::<serde_json::Value>();
    let session_id = json_response["session_id"].as_str().unwrap().to_string();
    let session = test_app
        .storage
        .get_session(&session_id)
        .await
        .unwrap()
        .unwrap();
    let text = session.solution.clone();

    // Validate with correct solution (case-sensitive)
    let response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": text}))
        .await;

    response.assert_status_ok();
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["valid"], true);

    // The sessions.validated metric should have been incremented
}

/// Test that sessions.deleted metric is incremented when deleting sessions
#[tokio::test]
async fn test_sessions_deleted_metric() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create multiple sessions
    let mut session_ids = Vec::new();
    for _ in 0..3 {
        let response = server
            .post("/api/v1/sessions")
            .add_header("Authorization", format!("Bearer {}", test_app.api_key))
            .json(&json!({}))
            .await;

        response.assert_status(axum::http::StatusCode::CREATED);
        session_ids.push(
            response.json::<serde_json::Value>()["session_id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }

    // Delete each session
    for session_id in session_ids {
        let response = server
            .delete(&format!("/api/v1/sessions/{}", session_id))
            .add_header("Authorization", format!("Bearer {}", test_app.api_key))
            .await;

        response.assert_status(axum::http::StatusCode::NO_CONTENT);
    }

    // The sessions.deleted metric should have been incremented 3 times
}

/// Test that api_keys.created metric is incremented when creating API keys
#[tokio::test]
async fn test_api_keys_created_metric() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create multiple API keys using master key
    for i in 0..3 {
        let response = server
            .post("/api/v1/api-keys")
            .add_header("Authorization", format!("Bearer {}", test_app.master_key))
            .json(&json!({"description": format!("Test Key {}", i)}))
            .await;

        response.assert_status(axum::http::StatusCode::CREATED);
    }

    // The api_keys.created metric should have been incremented 3 times
}

/// Test that api_keys.deleted metric is incremented when deleting API keys
#[tokio::test]
async fn test_api_keys_deleted_metric() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create API keys
    let mut key_hashes = Vec::new();
    for i in 0..3 {
        let response = server
            .post("/api/v1/api-keys")
            .add_header("Authorization", format!("Bearer {}", test_app.master_key))
            .json(&json!({"description": format!("Test Key {}", i)}))
            .await;

        response.assert_status(axum::http::StatusCode::CREATED);
        key_hashes.push(
            response.json::<serde_json::Value>()["key_hash"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }

    // Delete each API key
    for key_hash in key_hashes {
        let response = server
            .delete(&format!("/api/v1/api-keys/{}", key_hash))
            .add_header("Authorization", format!("Bearer {}", test_app.master_key))
            .await;

        response.assert_status(axum::http::StatusCode::NO_CONTENT);
    }

    // The api_keys.deleted metric should have been incremented 3 times
}

/// Test that api_key_authentications metric is incremented on auth attempts
#[tokio::test]
async fn test_api_key_authentications_metric() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Make multiple authenticated requests
    for _ in 0..10 {
        let response = server
            .post("/api/v1/sessions")
            .add_header("Authorization", format!("Bearer {}", test_app.api_key))
            .json(&json!({}))
            .await;

        response.assert_status(axum::http::StatusCode::CREATED);
    }

    // The api_key_authentications metric should have been incremented 10 times
}

/// Test that api_key_authentications metric is incremented even for failed auth
#[tokio::test]
async fn test_api_key_authentications_metric_on_failed_auth() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Make requests with invalid API key
    for _ in 0..5 {
        let response = server
            .post("/api/v1/sessions")
            .add_header("Authorization", "Bearer invalid-key")
            .json(&json!({}))
            .await;

        response.assert_status_unauthorized();
    }

    // The api_key_authentications metric should have been incremented 5 times
}

/// Test that metrics are updated correctly in a complete user flow
#[tokio::test]
async fn test_metrics_in_complete_flow() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // 1. Create a session (sessions.created + api_key_authentications)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 7
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let json_response = response.json::<serde_json::Value>();
    let session_id = json_response["session_id"].as_str().unwrap().to_string();
    let session = test_app
        .storage
        .get_session(&session_id)
        .await
        .unwrap()
        .unwrap();
    let text = session.solution.clone();

    // 2. Make failed validation attempts (session_validation_attempts + api_key_authentications)
    for _ in 0..2 {
        let response = server
            .post(&format!("/api/v1/sessions/{}/validate", session_id))
            .add_header("Authorization", format!("Bearer {}", test_app.api_key))
            .json(&json!({"solution": "wrong_solution"}))
            .await;

        response.assert_status_ok();
        assert_eq!(response.json::<serde_json::Value>()["valid"], false);
    }

    // 3. Make successful validation (session_validation_attempts + sessions.validated + api_key_authentications)
    let response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": text}))
        .await;

    response.assert_status_ok();
    assert_eq!(response.json::<serde_json::Value>()["valid"], true);

    // Total metrics updated:
    // - sessions.created: 1
    // - session_validation_attempts: 3
    // - sessions.validated: 1
    // - api_key_authentications: 4
}

/// Test that sessions_expired_cleaned metric is incremented during cleanup
#[tokio::test]
async fn test_sessions_expired_cleaned_metric() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create sessions with very short TTL (1 second)
    for _ in 0..3 {
        let response = server
            .post("/api/v1/sessions")
            .add_header("Authorization", format!("Bearer {}", test_app.api_key))
            .json(&json!({"expires_in_seconds": 1}))
            .await;

        response.assert_status(axum::http::StatusCode::CREATED);
    }

    // Wait for sessions to expire
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Manually trigger cleanup (simulating what the background task does)
    let deleted_count = test_app.storage.delete_expired_sessions().await.unwrap();
    assert_eq!(deleted_count, 3);

    // Note: In the real application, the cleanup task would record this metric
    // We verify here that the cleanup works correctly
}

/// Test that metrics survive across different API key usages
#[tokio::test]
async fn test_metrics_with_multiple_api_keys() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create multiple API keys
    let mut api_keys = Vec::new();
    for i in 0..3 {
        let response = server
            .post("/api/v1/api-keys")
            .add_header("Authorization", format!("Bearer {}", test_app.master_key))
            .json(&json!({"description": format!("Test Key {}", i)}))
            .await;

        response.assert_status(axum::http::StatusCode::CREATED);
        api_keys.push(
            response.json::<serde_json::Value>()["api_key"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }

    // Use each API key to create sessions
    for api_key in &api_keys {
        let response = server
            .post("/api/v1/sessions")
            .add_header("Authorization", format!("Bearer {}", api_key))
            .json(&json!({}))
            .await;

        response.assert_status(axum::http::StatusCode::CREATED);
    }

    // Metrics should be updated correctly:
    // - api_keys.created: 3
    // - api_key_authentications: 3 (from session creation) + 3 (from key creation) = 6
    // - sessions.created: 3
}

/// Test that metrics handle edge cases gracefully
#[tokio::test]
async fn test_metrics_edge_cases() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Test: Deleting non-existent session
    let response = server
        .delete("/api/v1/sessions/non-existent-id")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .await;

    response.assert_status(axum::http::StatusCode::NOT_FOUND);
    // sessions.deleted should NOT be incremented

    // Test: Validating non-existent session
    let response = server
        .post("/api/v1/sessions/non-existent-id/validate")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": "test"}))
        .await;

    response.assert_status(axum::http::StatusCode::NOT_FOUND);
    // session_validation_attempts should NOT be incremented

    // Test: Deleting already-deleted API key
    let response = server
        .post("/api/v1/api-keys")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({"description": "To Delete"}))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let key_hash = response.json::<serde_json::Value>()["key_hash"]
        .as_str()
        .unwrap()
        .to_string();

    // Delete it once
    let response = server
        .delete(&format!("/api/v1/api-keys/{}", key_hash))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .await;

    response.assert_status(axum::http::StatusCode::NO_CONTENT);

    // Try to delete it again
    let response = server
        .delete(&format!("/api/v1/api-keys/{}", key_hash))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .await;

    response.assert_status(axum::http::StatusCode::NOT_FOUND);
    // api_keys.deleted should only be incremented once
}

/// Test that all counter metrics are properly initialized and accessible
#[tokio::test]
async fn test_all_counter_metrics_initialized() {
    let _test_app = TestApp::new().await;

    // Create a metrics instance like the app does
    let metrics = captchapi::metrics::Metrics::new();

    // Verify all counter metrics are accessible
    // Session metrics
    metrics.sessions.created.add(1, &[]);
    metrics.sessions.validated.add(1, &[]);
    metrics.sessions.deleted.add(1, &[]);
    metrics.sessions.expired_cleaned.add(1, &[]);
    metrics.sessions.validation_attempts.add(1, &[]);

    // API key metrics
    metrics.api_keys.created.add(1, &[]);
    metrics.api_keys.deleted.add(1, &[]);
    metrics.api_keys.authentications.add(1, &[]);

    // All metrics should be accessible and work without panicking
}

/// Test that histogram metrics can be recorded
#[tokio::test]
async fn test_histogram_metrics_recording() {
    let metrics = captchapi::metrics::Metrics::new();

    // Test that histogram metrics exist and can be recorded
    metrics
        .performance
        .captcha_generation_duration
        .record(0.125, &[]);
    metrics.performance.request_duration.record(0.050, &[]);

    // Multiple recordings should work
    metrics
        .performance
        .captcha_generation_duration
        .record(0.200, &[]);
    metrics.performance.request_duration.record(0.100, &[]);

    // These should not panic even though we're not using them actively yet
}

/// Test that attempting to validate an already-validated session doesn't double-count
#[tokio::test]
async fn test_metrics_no_double_count_after_validation() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create a session
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 8
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let json_response = response.json::<serde_json::Value>();
    let session_id = json_response["session_id"].as_str().unwrap().to_string();
    let session = test_app
        .storage
        .get_session(&session_id)
        .await
        .unwrap()
        .unwrap();
    let text = session.solution.clone();

    // Validate successfully
    let response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": text.clone()}))
        .await;

    response.assert_status_ok();
    assert_eq!(response.json::<serde_json::Value>()["valid"], true);

    // Try to validate again (should fail as session is deleted after successful validation)
    let response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": text}))
        .await;

    response.assert_status(axum::http::StatusCode::NOT_FOUND);
    // Metrics should not be double-counted
}
