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

    // 2. Get the image (public endpoint)
    let image_response = server
        .get(&format!("/api/v1/sessions/{}/image", session_id))
        .await;

    image_response.assert_status_ok();
    let image_body: serde_json::Value = image_response.json();
    assert!(image_body.get("image").is_some());

    let image_data_uri = image_body["image"].as_str().unwrap();
    assert!(
        image_data_uri.starts_with("data:image/jpeg;base64,"),
        "Image should be JPEG data URI"
    );

    // Fully validate the base64 JPEG data
    let base64_data = image_data_uri
        .strip_prefix("data:image/jpeg;base64,")
        .expect("Should have correct data URI prefix");

    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, base64_data);
    assert!(
        decoded.is_ok(),
        "Should decode valid base64. Error: {:?}",
        decoded.as_ref().err()
    );

    let decoded_bytes = decoded.unwrap();
    let jpeg_signature: [u8; 3] = [255, 216, 255];
    assert!(
        decoded_bytes.starts_with(&jpeg_signature),
        "Should be valid JPEG file"
    );

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

    // 4. Try to get the session again (should be deleted)
    let image_response2 = server
        .get(&format!("/api/v1/sessions/{}/image", session_id))
        .await;

    image_response2.assert_status_not_found();
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
    let image_response = server
        .get(&format!("/api/v1/sessions/{}/image", session_id))
        .await;

    image_response.assert_status_ok();
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
    let image_response = server
        .get(&format!("/api/v1/sessions/{}/image", session_id))
        .await;

    image_response.assert_status_not_found();
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

    let response = server.get("/api/v1/sessions/nonexistent-id/image").await;

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
