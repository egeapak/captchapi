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

/// An app whose `captcha_compression` was set on the command line, as a deployment would.
fn pinned_app(app: &TestApp) -> axum::Router {
    let handle = captchapi::config::ConfigHandle::from_static_with_cli(
        captchapi::config::Config {
            master_api_key: app.master_key.clone(),
            ..captchapi::config::Config::for_test()
        },
        &[("CAPTCHA_COMPRESSION", "70")],
    )
    .expect("fixture config must resolve");
    app.build_app_with_handle(handle)
}

#[tokio::test]
async fn test_get_config_reports_the_layer_each_value_came_from() {
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(pinned_app(&app));

    let response = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();

    assert_eq!(body["config"]["captcha_compression"]["source"], "cli");
    assert_eq!(
        body["config"]["max_validation_attempts"]["source"],
        "default"
    );
}

#[tokio::test]
async fn test_a_value_set_on_the_command_line_is_not_editable() {
    // Reloadable, but this process was handed an explicit value, so an override would last
    // only until the next reload. The API says so up front rather than accepting and losing it.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(pinned_app(&app));

    let response = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;
    let body: serde_json::Value = response.json();

    assert_eq!(body["config"]["captcha_compression"]["reloadable"], true);
    assert_eq!(body["config"]["captcha_compression"]["editable"], false);
    // A live field nobody pinned stays editable, so this is provenance and not a blanket off.
    assert_eq!(body["config"]["max_validation_attempts"]["editable"], true);
    // Boot fields were never editable and still are not.
    assert_eq!(body["config"]["server_port"]["editable"], false);
}

#[tokio::test]
async fn test_patching_a_pinned_field_is_refused_with_its_own_error_code() {
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(pinned_app(&app));

    let response = server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .json(&json!({ "captcha_compression": 90 }))
        .await;

    response.assert_status(axum::http::StatusCode::CONFLICT);
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "config_pinned");
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("command line"), "{message}");
    assert!(message.contains("captcha_compression"), "{message}");

    // And the running value is untouched.
    let after = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;
    let after: serde_json::Value = after.json();
    assert_eq!(after["config"]["captcha_compression"]["value"], "70");
    assert!(after["overrides"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_an_unpinned_field_is_still_patchable_on_a_pinned_server() {
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(pinned_app(&app));

    let response = server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .json(&json!({ "max_validation_attempts": 7 }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["config"]["max_validation_attempts"]["value"], "7");
    // The admin API set it, so it reports as `admin` — not `cli`, which would pin it against
    // ever being patched again.
    assert_eq!(body["config"]["max_validation_attempts"]["source"], "admin");
    assert_eq!(body["config"]["max_validation_attempts"]["editable"], true);
}

#[tokio::test]
async fn test_get_config_describes_every_parameter() {
    // The console renders these, so a client never ships its own copy of the documentation
    // and lets it drift from what the binary actually does.
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let config = body["config"].as_object().unwrap();

    assert!(!config.is_empty());
    for (field, entry) in config {
        let description = entry["description"]
            .as_str()
            .unwrap_or_else(|| panic!("{field} has no description"));
        assert!(
            description.ends_with('.'),
            "{field}: expected a sentence, got {description:?}"
        );
    }

    // Secrets are described too — the description says what the parameter is for, which is
    // exactly what an operator needs when the value itself is redacted.
    assert!(config["master_api_key"]["description"]
        .as_str()
        .unwrap()
        .contains("administrative access"));
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
    assert_eq!(body["overrides"], json!(["captcha_compression"]));
}

#[tokio::test]
async fn test_overrides_can_be_fed_straight_back_into_patch() {
    // The whole API must speak one key form. `overrides` previously returned canonical env
    // keys (CAPTCHA_COMPRESSION) while PATCH accepted field names (captcha_compression), so
    // the obvious "revert what an operator changed" loop died with 400 invalid_config and
    // `config[overrides[0]]` was always undefined.
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "captcha_compression": 90, "max_validation_attempts": 5 }))
        .await
        .assert_status_ok();

    let listed = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;
    listed.assert_status_ok();
    let body: serde_json::Value = listed.json();

    let overrides = body["overrides"].as_array().unwrap().clone();
    assert_eq!(overrides.len(), 2, "{overrides:?}");

    for name in overrides {
        let name = name.as_str().unwrap();

        // Every name in `overrides` must key into the `config` map...
        assert!(
            !body["config"][name].is_null(),
            "`{name}` from overrides is not a key of the config map"
        );

        // ...and must be accepted by PATCH.
        server
            .patch("/api/v1/admin/config")
            .add_header("Authorization", format!("Bearer {}", app.master_key))
            .json(&json!({ name: body["config"][name]["value"].as_str().unwrap() }))
            .await
            .assert_status_ok();
    }
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

// ── ADMIN_CONFIG_WRITE=false ─────────────────────────────────────────────────

#[tokio::test]
async fn test_patch_is_forbidden_when_runtime_writes_are_disabled() {
    // The hardening opt-out: deployments that want file-driven reload but no remote mutation.
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app_with_config(captchapi::config::Config {
        master_api_key: app.master_key.clone(),
        admin_config_write: false,
        ..captchapi::config::Config::for_test()
    }));

    let response = server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .json(&json!({ "captcha_compression": 90 }))
        .await;

    // 403, not 401: the caller authenticated fine, the operation is disabled by policy.
    response.assert_status_forbidden();
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "forbidden");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("ADMIN_CONFIG_WRITE"));
}

#[tokio::test]
async fn test_reads_and_reload_still_work_when_writes_are_disabled() {
    // Turning off writes must not turn off the ability to inspect or to reload from files.
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app_with_config(captchapi::config::Config {
        master_api_key: app.master_key.clone(),
        admin_config_write: false,
        ..captchapi::config::Config::for_test()
    }));

    server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await
        .assert_status_ok();

    server
        .post("/api/v1/admin/config/reload")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await
        .assert_status_ok();
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
