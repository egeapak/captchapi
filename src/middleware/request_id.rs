use axum::{extract::Request, http::HeaderValue, middleware::Next, response::Response};
use uuid::Uuid;

/// Header name for request ID
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Middleware that generates or extracts a request ID and adds it to the response
///
/// If the incoming request has an X-Request-ID header, it will be reused.
/// Otherwise, a new UUID v4 will be generated.
///
/// The request ID is:
/// - Added to the response headers as X-Request-ID
/// - Can be used for log correlation and distributed tracing
pub async fn request_id_middleware(request: Request, next: Next) -> Response {
    // Check if request already has a request ID
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Process the request
    let mut response = next.run(request).await;

    // Add request ID to response headers
    if let Ok(header_value) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(REQUEST_ID_HEADER, header_value);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    async fn test_handler() -> &'static str {
        "OK"
    }

    #[tokio::test]
    async fn test_request_id_generated_when_missing() {
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn(request_id_middleware));

        let response = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let request_id = response.headers().get(REQUEST_ID_HEADER);
        assert!(request_id.is_some(), "Response should have request ID");

        let request_id_str = request_id.unwrap().to_str().unwrap();
        assert!(!request_id_str.is_empty(), "Request ID should not be empty");

        // Verify it's a valid UUID
        assert!(
            Uuid::parse_str(request_id_str).is_ok(),
            "Request ID should be a valid UUID"
        );
    }

    #[tokio::test]
    async fn test_request_id_preserved_when_provided() {
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn(request_id_middleware));

        let custom_id = "custom-request-id-12345";

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header(REQUEST_ID_HEADER, custom_id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let request_id = response
            .headers()
            .get(REQUEST_ID_HEADER)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(
            request_id, custom_id,
            "Response should preserve the provided request ID"
        );
    }

    #[tokio::test]
    async fn test_request_id_generated_when_empty() {
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn(request_id_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header(REQUEST_ID_HEADER, "")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let request_id = response
            .headers()
            .get(REQUEST_ID_HEADER)
            .unwrap()
            .to_str()
            .unwrap();

        assert!(
            !request_id.is_empty(),
            "Empty request ID should be replaced"
        );
        assert!(
            Uuid::parse_str(request_id).is_ok(),
            "New request ID should be a valid UUID"
        );
    }
}
