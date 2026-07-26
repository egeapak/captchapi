//! The admin console: a static page, served from the binary.
//!
//! Size probe. The assets are embedded with `include_bytes!` rather than served from disk,
//! because the deployment story is a single static binary in a distroless/scratch image —
//! a `ServeDir` would need files staged alongside it and would add `tower-http/fs`.
//!
//! Unauthenticated on purpose: this is the page that *asks* for the master key. It contains
//! no secrets and no configuration, only the markup and the fetch calls; every byte of data
//! it displays comes from `/api/v1/admin/config`, which is behind the master-key middleware.

use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

const INDEX_HTML: &[u8] = include_bytes!("../../assets/admin/index.html");
const APP_JS: &[u8] = include_bytes!("../../assets/admin/app.js");

/// Serve the console at `/admin`.
///
/// Absolute paths rather than a `nest`, because `nest("/admin", route("/"))` answers `/admin`
/// but 404s on `/admin/` — and an operator typing the URL will produce either one.
pub fn admin_ui_routes() -> Router {
    let index = || async { asset(INDEX_HTML, "text/html; charset=utf-8") };
    Router::new()
        .route("/admin", get(index))
        .route("/admin/", get(index))
        .route(
            "/admin/app.js",
            get(|| async { asset(APP_JS, "text/javascript; charset=utf-8") }),
        )
}

/// A static response with a locked-down CSP.
///
/// The page holds the master key in a JavaScript variable for the life of the tab, so an
/// injected script would be able to read it. `default-src 'none'` plus `script-src 'self'`
/// means only the two assets above can run: no inline handlers, no third-party origins,
/// no `connect-src` other than this one.
fn asset(body: &'static [u8], content_type: &'static str) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(
                    "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; \
                     connect-src 'self'; form-action 'none'; frame-ancestors 'none'; \
                     base-uri 'none'",
                ),
            ),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn get_path(path: &str) -> Response {
        admin_ui_routes()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_both_spellings_of_the_index_are_served() {
        for path in ["/admin", "/admin/"] {
            let res = get_path(path).await;
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            assert_eq!(
                res.headers()[header::CONTENT_TYPE],
                "text/html; charset=utf-8"
            );
        }
    }

    #[tokio::test]
    async fn test_the_script_is_served_as_javascript() {
        let res = get_path("/admin/app.js").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
    }

    #[tokio::test]
    async fn test_every_asset_carries_the_csp() {
        // The page holds the master key in memory, so a missing CSP is a real weakening
        // rather than a cosmetic one.
        for path in ["/admin", "/admin/", "/admin/app.js"] {
            let res = get_path(path).await;
            let csp = res.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .to_string();
            assert!(csp.contains("default-src 'none'"), "{path}: {csp}");
            assert!(csp.contains("script-src 'self'"), "{path}: {csp}");
            assert!(csp.contains("frame-ancestors 'none'"), "{path}: {csp}");
            assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
        }
    }

    #[tokio::test]
    async fn test_the_console_ships_no_third_party_code() {
        // The size argument for hand-writing this page only holds if it stays dependency-free,
        // and a CDN tag would also defeat `script-src 'self'`.
        let html = std::str::from_utf8(INDEX_HTML).unwrap();
        assert!(!html.contains("//cdn."), "{html}");
        assert!(!html.contains("https://"), "{html}");
        assert_eq!(
            html.matches("<script").count(),
            1,
            "the console loads exactly one script, its own"
        );
    }

    #[tokio::test]
    async fn test_the_console_holds_no_credential() {
        // It is served unauthenticated, so anything baked into it is public.
        let bytes = [INDEX_HTML, APP_JS].concat();
        let text = std::str::from_utf8(&bytes).unwrap();
        for needle in [
            "MASTER_API_KEY",
            "API_KEY_SALT",
            "Bearer sk",
            "localStorage",
        ] {
            assert!(!text.contains(needle), "console must not contain {needle}");
        }
    }
}
