mod common;

use axum_test::TestServer;
use common::TestApp;
use serde_json::json;

#[tokio::test]
async fn test_health_check() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server.get("/health").await;

    response.assert_status_ok();
    response.assert_json(&json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION")
    }));
}

#[tokio::test]
async fn test_create_session_without_auth_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .json(&json!({
            "difficulty": 5
        }))
        .await;

    response.assert_status_unauthorized();
}

#[tokio::test]
async fn test_create_session_with_auth_succeeds() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "difficulty": 5,
            "width": 220,
            "height": 120
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = response.json();

    assert!(body.get("session_id").is_some());
    assert!(body.get("expires_at").is_some());
    assert!(body.get("created_at").is_some());
}

#[tokio::test]
async fn test_complete_session_flow() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // 1. Create session with custom text
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "text": "ABC123",
            "difficulty": 5,
            "expires_in_seconds": 300
        }))
        .await;

    create_response.assert_status(axum::http::StatusCode::CREATED);
    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // 2. Get session details (public endpoint)
    let details_response = server
        .get(&format!("/api/v1/sessions/{}", session_id))
        .await;

    details_response.assert_status_ok();
    let details_body: serde_json::Value = details_response.json();
    assert_eq!(details_body["session_id"], session_id);
    assert!(details_body.get("created_at").is_some());
    assert!(details_body.get("expires_at").is_some());
    assert_eq!(details_body["difficulty"], 5);
    assert_eq!(details_body["attempt_count"], 0);

    // 3. Validate with correct solution
    let validate_response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "solution": "ABC123"
        }))
        .await;

    validate_response.assert_status_ok();
    let validate_body: serde_json::Value = validate_response.json();
    assert_eq!(validate_body["valid"], true);

    // 4. Try to get the session details again (should be deleted)
    let details_response2 = server
        .get(&format!("/api/v1/sessions/{}", session_id))
        .await;

    details_response2.assert_status_not_found();
}

#[tokio::test]
async fn test_validate_session_with_wrong_solution() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create session
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "text": "CORRECT",
        }))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Validate with wrong solution
    let validate_response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "solution": "WRONG"
        }))
        .await;

    validate_response.assert_status_ok();
    let validate_body: serde_json::Value = validate_response.json();
    assert_eq!(validate_body["valid"], false);

    // Session should still exist
    let details_response = server
        .get(&format!("/api/v1/sessions/{}", session_id))
        .await;

    details_response.assert_status_ok();
}

#[tokio::test]
async fn test_delete_session() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create session
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({}))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Delete session
    let delete_response = server
        .delete(&format!("/api/v1/sessions/{}", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .await;

    delete_response.assert_status(axum::http::StatusCode::NO_CONTENT);

    // Session should no longer exist
    let details_response = server
        .get(&format!("/api/v1/sessions/{}", session_id))
        .await;

    details_response.assert_status_not_found();
}

#[tokio::test]
async fn test_create_session_with_invalid_parameters() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Difficulty out of range
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "difficulty": 15
        }))
        .await;

    response.assert_status_bad_request();
}

#[tokio::test]
async fn test_get_nonexistent_session() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server.get("/api/v1/sessions/nonexistent-id").await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_get_binary_image() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create session with custom text
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "text": "BINARY",
            "expires_in_seconds": 300
        }))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Get the binary image
    let image_response = server
        .get(&format!("/api/v1/sessions/{}/image.jpeg", session_id))
        .await;

    image_response.assert_status_ok();

    // Check headers
    let headers = image_response.headers();
    assert_eq!(
        headers.get("content-type").unwrap(),
        "image/jpeg",
        "Should have correct content type"
    );
    assert!(headers.get("etag").is_some(), "Should have ETag header");
    assert!(
        headers.get("cache-control").is_some(),
        "Should have Cache-Control header"
    );
    assert!(
        headers.get("expires").is_some(),
        "Should have Expires header"
    );

    // Verify it's actual binary JPEG data
    let image_bytes = image_response.as_bytes();
    assert!(!image_bytes.is_empty(), "Image data should not be empty");

    // Verify JPEG signature
    let jpeg_signature: [u8; 3] = [255, 216, 255];
    assert!(
        image_bytes.starts_with(&jpeg_signature),
        "Should be valid JPEG file"
    );
    assert!(
        image_bytes.len() > 1000,
        "JPEG should have substantial data"
    );
}

#[tokio::test]
async fn test_create_session_max_ttl() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create session with max TTL (3600 seconds)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "expires_in_seconds": 3600
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
}

#[tokio::test]
async fn test_create_session_exceeds_max_ttl() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Try to create session with TTL exceeding max (3601 > 3600)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "expires_in_seconds": 3601
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("cannot exceed 3600"));
}

#[tokio::test]
async fn test_create_session_difficulty_boundaries() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Test difficulty = 1 (minimum)
    let response1 = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"difficulty": 1}))
        .await;
    response1.assert_status(axum::http::StatusCode::CREATED);

    // Test difficulty = 10 (maximum)
    let response2 = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"difficulty": 10}))
        .await;
    response2.assert_status(axum::http::StatusCode::CREATED);

    // Test difficulty = 0 (below minimum)
    let response3 = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"difficulty": 0}))
        .await;
    response3.assert_status_bad_request();

    // Test difficulty = 11 (above maximum)
    let response4 = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"difficulty": 11}))
        .await;
    response4.assert_status_bad_request();
}

#[tokio::test]
async fn test_validation_case_insensitive() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create session with mixed case text
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"text": "AbC123"}))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Test lowercase
    let response1 = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": "abc123"}))
        .await;
    response1.assert_status_ok();
    let body1: serde_json::Value = response1.json();
    assert_eq!(body1["valid"], true);
}

#[tokio::test]
async fn test_validate_three_failed_attempts_deletes_session() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create session
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"text": "CORRECT"}))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // First failed attempt
    let response1 = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": "WRONG1"}))
        .await;
    let body1: serde_json::Value = response1.json();
    assert_eq!(body1["valid"], false);

    // Second failed attempt
    let response2 = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": "WRONG2"}))
        .await;
    let body2: serde_json::Value = response2.json();
    assert_eq!(body2["valid"], false);

    // Third failed attempt - should delete session
    let response3 = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": "WRONG3"}))
        .await;
    let body3: serde_json::Value = response3.json();
    assert_eq!(body3["valid"], false);

    // Fourth attempt - attempt_count is now 3 (>= max), should return valid=false and delete
    let response4 = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": "CORRECT"}))
        .await;
    response4.assert_status_ok();
    let body4: serde_json::Value = response4.json();
    assert_eq!(body4["valid"], false); // Returns false even with correct solution

    // Fifth attempt should get 404 - session is now deleted
    let response5 = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": "CORRECT"}))
        .await;
    response5.assert_status_not_found();
}

#[tokio::test]
async fn test_binary_image_cache_headers() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create session with specific TTL
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"expires_in_seconds": 300}))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Get binary image
    let image_response = server
        .get(&format!("/api/v1/sessions/{}/image.jpeg", session_id))
        .await;

    // Check cache headers
    let headers = image_response.headers();

    // ETag should be the session ID
    let etag = headers.get("etag").unwrap();
    assert!(etag.to_str().unwrap().contains(session_id));

    // Cache-Control should have max-age
    let cache_control = headers.get("cache-control").unwrap().to_str().unwrap();
    assert!(cache_control.contains("public"));
    assert!(cache_control.contains("max-age"));

    // Extract max-age value
    let max_age_str = cache_control
        .split("max-age=")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap();
    let max_age: i64 = max_age_str.parse().unwrap();

    // Should be roughly 300 seconds (allow some drift)
    assert!((295..=300).contains(&max_age), "max-age should be ~300s");

    // Expires header should be present
    assert!(headers.get("expires").is_some());
}

#[tokio::test]
async fn test_create_session_with_custom_dimensions() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "width": 300,
            "height": 150,
            "dark_mode": true
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
}

#[tokio::test]
async fn test_delete_nonexistent_session() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .delete("/api/v1/sessions/nonexistent-id")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_validate_nonexistent_session() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions/nonexistent-id/validate")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": "TEST"}))
        .await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_create_session_with_master_key_succeeds() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "difficulty": 5,
            "width": 220,
            "height": 120
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = response.json();

    assert!(body.get("session_id").is_some());
    assert!(body.get("expires_at").is_some());
    assert!(body.get("created_at").is_some());
}

#[tokio::test]
async fn test_validate_session_with_master_key_succeeds() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create session using regular API key
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "text": "MASTER",
            "difficulty": 5,
            "expires_in_seconds": 300
        }))
        .await;

    create_response.assert_status(axum::http::StatusCode::CREATED);
    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Validate session using master key
    let validate_response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "solution": "MASTER"
        }))
        .await;

    validate_response.assert_status_ok();
    let validate_body: serde_json::Value = validate_response.json();
    assert_eq!(validate_body["valid"], true);
}

#[tokio::test]
async fn test_delete_session_with_master_key_succeeds() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Create session using regular API key
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({}))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Delete session using master key
    let delete_response = server
        .delete(&format!("/api/v1/sessions/{}", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .await;

    delete_response.assert_status(axum::http::StatusCode::NO_CONTENT);

    // Session should no longer exist
    let details_response = server
        .get(&format!("/api/v1/sessions/{}", session_id))
        .await;

    details_response.assert_status_not_found();
}

#[tokio::test]
async fn test_complete_session_flow_with_master_key() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // 1. Create session with master key
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "text": "MASTER123",
            "difficulty": 5,
            "expires_in_seconds": 300
        }))
        .await;

    create_response.assert_status(axum::http::StatusCode::CREATED);
    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // 2. Get session details (public endpoint)
    let details_response = server
        .get(&format!("/api/v1/sessions/{}", session_id))
        .await;

    details_response.assert_status_ok();
    let details_body: serde_json::Value = details_response.json();
    assert_eq!(details_body["session_id"], session_id);
    assert_eq!(details_body["difficulty"], 5);
    assert_eq!(details_body["attempt_count"], 0);

    // 3. Validate with correct solution using master key
    let validate_response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "solution": "MASTER123"
        }))
        .await;

    validate_response.assert_status_ok();
    let validate_body: serde_json::Value = validate_response.json();
    assert_eq!(validate_body["valid"], true);

    // 4. Session should be deleted after successful validation
    let details_response2 = server
        .get(&format!("/api/v1/sessions/{}", session_id))
        .await;

    details_response2.assert_status_not_found();
}

#[tokio::test]
async fn test_create_session_with_empty_text_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "text": ""
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
    assert!(body["message"].as_str().unwrap().contains("empty"));
}

#[tokio::test]
async fn test_create_session_with_text_too_long_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "text": "ABCDEFGHIJKLMNOPQRSTUVWXYZ"  // 26 characters, exceeds 20 limit
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
    assert!(body["message"].as_str().unwrap().contains("20 characters"));
}

#[tokio::test]
async fn test_create_session_with_non_alphanumeric_text_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "text": "ABC@123"  // Contains special character
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
    assert!(body["message"].as_str().unwrap().contains("alphanumeric"));
}

#[tokio::test]
async fn test_create_session_with_width_too_small_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "width": 30  // Below minimum of 50
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
    assert!(body["message"].as_str().unwrap().contains("width"));
}

#[tokio::test]
async fn test_create_session_with_width_too_large_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "width": 2000  // Above maximum of 1000
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
    assert!(body["message"].as_str().unwrap().contains("width"));
}

#[tokio::test]
async fn test_create_session_with_height_too_small_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "height": 20  // Below minimum of 30
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
    assert!(body["message"].as_str().unwrap().contains("height"));
}

#[tokio::test]
async fn test_create_session_with_height_too_large_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "height": 600  // Above maximum of 500
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
    assert!(body["message"].as_str().unwrap().contains("height"));
}

#[tokio::test]
async fn test_create_session_with_valid_boundary_dimensions() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Test minimum boundaries
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "width": 50,
            "height": 30
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);

    // Test maximum boundaries
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "width": 1000,
            "height": 500
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
}

#[tokio::test]
async fn test_create_session_with_valid_text_boundary() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Test single character (minimum)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "text": "A"
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);

    // Test exactly 20 characters (maximum)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "text": "ABCDEFGHIJ1234567890"  // Exactly 20 characters
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
}
