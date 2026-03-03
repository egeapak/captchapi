mod common;

use axum_test::TestServer;
use common::TestApp;
use serde_json::json;

#[tokio::test]
async fn test_health_check() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    let response = server.get("/health").await;

    response.assert_status_ok();
    response.assert_json(&json!({
        "status": "healthy"
    }));
}

#[tokio::test]
async fn test_create_session_without_auth_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
    assert!(body.get("text").is_none());
}

#[tokio::test]
async fn test_complete_session_flow() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // 1. Create session with custom length
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 6,
            "difficulty": 5,
            "expires_in_seconds": 300
        }))
        .await;

    create_response.assert_status(axum::http::StatusCode::CREATED);
    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Get solution from the database (text is no longer in the response)
    let session = test_app
        .storage
        .get_session(session_id)
        .await
        .unwrap()
        .unwrap();
    let text = &session.solution;

    // Verify text has correct length
    assert_eq!(text.len(), 6);

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
            "solution": text
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
    let server = TestServer::new(app);

    // Create session
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 7,
        }))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Validate with wrong solution
    let validate_response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "solution": "WRONG11"
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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

    let response = server.get("/api/v1/sessions/nonexistent-id").await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_get_binary_image() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create session with custom length
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 6,
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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
async fn test_validation_case_sensitive() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create session
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"length": 6}))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();
    let session = test_app
        .storage
        .get_session(session_id)
        .await
        .unwrap()
        .unwrap();
    let text = session.solution.clone();

    // Test exact match (should be valid)
    let response1 = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": text.clone()}))
        .await;
    response1.assert_status_ok();
    let body1: serde_json::Value = response1.json();
    assert_eq!(body1["valid"], true);

    // Create another session to test case mismatch
    let create_response2 = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"length": 6}))
        .await;

    let create_body2: serde_json::Value = create_response2.json();
    let session_id2 = create_body2["session_id"].as_str().unwrap();
    let session2 = test_app
        .storage
        .get_session(session_id2)
        .await
        .unwrap()
        .unwrap();
    let text2 = session2.solution.clone();

    // Test with different case (should be invalid if text contains letters)
    let text2_swapped_case: String = text2
        .chars()
        .map(|c| {
            if c.is_uppercase() {
                c.to_lowercase().to_string()
            } else {
                c.to_uppercase().to_string()
            }
        })
        .collect();

    // Only test if the swapped case is actually different
    if text2 != text2_swapped_case {
        let response2 = server
            .post(&format!("/api/v1/sessions/{}/validate", session_id2))
            .add_header("Authorization", format!("Bearer {}", test_app.api_key))
            .json(&json!({"solution": text2_swapped_case}))
            .await;
        response2.assert_status_ok();
        let body2: serde_json::Value = response2.json();
        assert_eq!(body2["valid"], false);
    }
}

#[tokio::test]
async fn test_validate_three_failed_attempts_deletes_session() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create session
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"length": 7}))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();
    let session = test_app
        .storage
        .get_session(session_id)
        .await
        .unwrap()
        .unwrap();
    let correct_text = session.solution.clone();

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
        .json(&json!({"solution": correct_text.clone()}))
        .await;
    response4.assert_status_ok();
    let body4: serde_json::Value = response4.json();
    assert_eq!(body4["valid"], false); // Returns false even with correct solution

    // Fifth attempt should get 404 - session is now deleted
    let response5 = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": correct_text.clone()}))
        .await;
    response5.assert_status_not_found();
}

#[tokio::test]
async fn test_binary_image_cache_headers() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
    assert!(body.get("text").is_none());
}

#[tokio::test]
async fn test_validate_session_with_master_key_succeeds() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create session using regular API key
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 6,
            "difficulty": 5,
            "expires_in_seconds": 300
        }))
        .await;

    create_response.assert_status(axum::http::StatusCode::CREATED);
    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();
    let session = test_app
        .storage
        .get_session(session_id)
        .await
        .unwrap()
        .unwrap();
    let text = session.solution.clone();

    // Validate session using master key
    let validate_response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "solution": text
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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

    // 1. Create session with master key
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.master_key))
        .json(&json!({
            "length": 9,
            "difficulty": 5,
            "expires_in_seconds": 300
        }))
        .await;

    create_response.assert_status(axum::http::StatusCode::CREATED);
    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();
    let session = test_app
        .storage
        .get_session(session_id)
        .await
        .unwrap()
        .unwrap();
    let text = session.solution.clone();

    // Verify text length
    assert_eq!(text.len(), 9);

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
            "solution": text
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
async fn test_create_session_with_length_zero_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 0
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
    assert!(body["message"].as_str().unwrap().contains("length"));
}

#[tokio::test]
async fn test_create_session_with_length_too_large_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 21  // Exceeds 20 limit
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
    assert!(body["message"].as_str().unwrap().contains("20 characters"));
}

#[tokio::test]
async fn test_create_session_with_negative_length_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": -1
        }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
}

#[tokio::test]
async fn test_create_session_with_width_too_small_fails() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
    let server = TestServer::new(app);

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
async fn test_create_session_with_valid_length_boundary() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Test length = 1 (minimum)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 1
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = response.json();
    let session_id = body["session_id"].as_str().unwrap();
    let session = test_app
        .storage
        .get_session(session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.solution.len(), 1);

    // Test length = 20 (maximum)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({
            "length": 20
        }))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = response.json();
    let session_id = body["session_id"].as_str().unwrap();
    let session = test_app
        .storage
        .get_session(session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.solution.len(), 20);
}

#[tokio::test]
async fn test_validate_session_solution_too_long() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // Create session
    let create_response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({}))
        .await;

    let create_body: serde_json::Value = create_response.json();
    let session_id = create_body["session_id"].as_str().unwrap();

    // Solution of exactly 101 characters should be rejected
    let long_solution = "a".repeat(101);
    let response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": long_solution}))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "invalid_parameters");
    assert!(body["message"].as_str().unwrap().contains("100 characters"));

    // Solution of exactly 100 characters should be accepted (not rejected by length check)
    let max_solution = "a".repeat(100);
    let response = server
        .post(&format!("/api/v1/sessions/{}/validate", session_id))
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"solution": max_solution}))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["valid"], false); // Wrong answer, but length is accepted
}

#[tokio::test]
async fn test_create_session_width_at_boundary_minus_one() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // width=49 should be rejected (boundary-1)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"width": 49}))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert!(body["message"].as_str().unwrap().contains("width"));

    // width=50 should succeed (exact boundary)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"width": 50}))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
}

#[tokio::test]
async fn test_create_session_height_at_boundary_minus_one() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // height=29 should be rejected (boundary-1)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"height": 29}))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert!(body["message"].as_str().unwrap().contains("height"));

    // height=30 should succeed (exact boundary)
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"height": 30}))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
}

#[tokio::test]
async fn test_create_session_with_compression_too_low() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // compression=0 is below the minimum of 1
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"compression": 0}))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert!(body["message"].as_str().unwrap().contains("compression"));
}

#[tokio::test]
async fn test_create_session_with_compression_too_high() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // compression=101 is above the maximum of 100
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"compression": 101}))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert!(body["message"].as_str().unwrap().contains("compression"));
}

#[tokio::test]
async fn test_create_session_with_valid_compression() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app);

    // compression=50 is within the valid 1-100 range
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"compression": 50}))
        .await;

    response.assert_status(axum::http::StatusCode::CREATED);
}
