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

// ── /api/v1/admin/config/stored ──────────────────────────────────────────────

#[tokio::test]
async fn test_stored_config_starts_empty() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .get("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert!(body["stored"].as_object().unwrap().is_empty());
    assert!(body["pending_restart"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_storing_a_live_field_applies_it_immediately() {
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app());

    let response = server
        .put("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .json(&json!({ "captcha_compression": 66 }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["stored"]["captcha_compression"], "66");
    assert!(body["pending_restart"].as_array().unwrap().is_empty());
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("all are in effect"));

    // ...and the running configuration moved with it.
    let after = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;
    let after: serde_json::Value = after.json();
    assert_eq!(after["config"]["captcha_compression"]["value"], "66");
    assert_eq!(after["config"]["captcha_compression"]["source"], "stored");
}

#[tokio::test]
async fn test_storing_a_boot_field_waits_for_a_restart() {
    // The distinction the whole feature rests on: it is persisted, but claiming it took effect
    // would be false — the rate limiter was built at startup.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app());

    let response = server
        .put("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .json(&json!({ "rate_limit_burst_size": 50 }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["stored"]["rate_limit_burst_size"], "50");
    assert_eq!(body["pending_restart"][0], "rate_limit_burst_size");
    assert!(body["message"].as_str().unwrap().contains("restart"));

    let after = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;
    let after: serde_json::Value = after.json();
    assert_eq!(
        after["config"]["rate_limit_burst_size"]["value"], "10",
        "the running value must not pretend to have changed"
    );
    assert_eq!(after["pending_restart"][0], "rate_limit_burst_size");
}

#[tokio::test]
async fn test_unstorable_fields_are_refused_with_their_own_code() {
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app());

    for field in [
        "api_key_salt",
        "master_api_key",
        "database_url",
        "otel_enabled",
    ] {
        let response = server
            .put("/api/v1/admin/config/stored")
            .add_header("Authorization", format!("Bearer {master_key}"))
            .json(&json!({ field: "x" }))
            .await;

        response.assert_status(StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json();
        assert_eq!(body["error"], "config_not_persistable", "{field}");
    }
}

#[tokio::test]
async fn test_an_invalid_value_is_refused_before_it_is_persisted() {
    // Writing first and validating afterwards would persist a configuration the server had
    // already refused, leaving the rollback machinery to undo it on the next restart.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app());

    let response = server
        .put("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .json(&json!({ "captcha_compression": "not-a-number" }))
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);

    let stored = server
        .get("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;
    let stored: serde_json::Value = stored.json();
    assert!(
        stored["stored"].as_object().unwrap().is_empty(),
        "nothing should have reached the database"
    );
}

#[tokio::test]
async fn test_deleting_a_stored_field_reverts_the_running_value() {
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app());

    server
        .put("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .json(&json!({ "captcha_compression": 66 }))
        .await
        .assert_status_ok();

    let response = server
        .delete("/api/v1/admin/config/stored/captcha_compression")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert!(body["stored"].as_object().unwrap().is_empty());

    let after = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;
    let after: serde_json::Value = after.json();
    assert_eq!(after["config"]["captcha_compression"]["value"], "40");
    assert_eq!(after["config"]["captcha_compression"]["source"], "default");
}

#[tokio::test]
async fn test_deleting_something_that_is_not_stored_is_an_error() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    let response = server
        .delete("/api/v1/admin/config/stored/captcha_compression")
        .add_header("Authorization", format!("Bearer {}", app.master_key))
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_a_stored_value_the_command_line_shadows_is_kept_and_reported() {
    // Not refused: it persists, and takes effect the moment the pin is dropped, which is the
    // migration path off command-line-driven configuration. It must simply never claim to be
    // in effect while something outranks it.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(pinned_app(&app));

    let response = server
        .put("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .json(&json!({ "captcha_compression": 66 }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["stored"]["captcha_compression"], "66");
    assert_eq!(body["shadowed"][0], "captcha_compression");

    let after = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;
    let after: serde_json::Value = after.json();
    let entry = &after["config"]["captcha_compression"];
    assert_eq!(entry["value"], "70", "the command line still wins");
    assert_eq!(entry["source"], "cli");
    assert_eq!(entry["shadowed_by"], "cli");
    assert_eq!(entry["storable"], true, "storable despite being pinned");
}

#[tokio::test]
async fn test_stored_writes_respect_admin_config_write() {
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app_with_config(captchapi::config::Config {
        master_api_key: master_key.clone(),
        admin_config_write: false,
        ..captchapi::config::Config::for_test()
    }));

    server
        .put("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .json(&json!({ "captcha_compression": 66 }))
        .await
        .assert_status(StatusCode::FORBIDDEN);

    // Reading the store is not a mutation, so it stays available.
    server
        .get("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn test_stored_endpoints_require_the_master_key() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    server
        .get("/api/v1/admin/config/stored")
        .await
        .assert_status_unauthorized();
    server
        .put("/api/v1/admin/config/stored")
        .json(&json!({ "captcha_compression": 66 }))
        .await
        .assert_status_unauthorized();
    server
        .delete("/api/v1/admin/config/stored/captcha_compression")
        .await
        .assert_status_unauthorized();
}

// ── POST /api/v1/admin/restart ───────────────────────────────────────────────

#[tokio::test]
async fn test_restart_is_refused_when_not_enabled() {
    // Off by default: a remote restart endpoint is an availability lever, and a denial-of-
    // service amplifier if the master key ever leaks.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app());

    let response = server
        .post("/api/v1/admin/restart")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;

    response.assert_status(StatusCode::FORBIDDEN);
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "restart_not_enabled");
}

#[tokio::test]
async fn test_restart_is_refused_when_the_process_cannot_restart_itself() {
    // Enabled in configuration, but this app was not built by `main`, so there is no handle on
    // the process. Refusing beats returning 200 and doing nothing.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app_with_config(captchapi::config::Config {
        master_api_key: master_key.clone(),
        admin_restart_enabled: true,
        ..captchapi::config::Config::for_test()
    }));

    let response = server
        .post("/api/v1/admin/restart")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await;

    response.assert_status(StatusCode::FORBIDDEN);
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"], "restart_not_enabled");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("not started in a way"));
}

#[tokio::test]
async fn test_restart_requires_the_master_key() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app());

    server
        .post("/api/v1/admin/restart")
        .await
        .assert_status_unauthorized();
}

#[tokio::test]
async fn test_a_stored_value_a_runtime_override_shadows_is_reported_as_shadowed() {
    // `Source::Admin` outranks even the command line, so a PATCH masks a stored value exactly
    // as an environment variable does. Reporting only the pinned layers meant this case came
    // back as `shadowed: []` while the server ran a different value entirely.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app());
    let auth = format!("Bearer {master_key}");

    server
        .put("/api/v1/admin/config/stored")
        .add_header("Authorization", auth.clone())
        .json(&json!({ "captcha_compression": 70 }))
        .await
        .assert_status_ok();

    server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", auth.clone())
        .json(&json!({ "captcha_compression": 90 }))
        .await
        .assert_status_ok();

    let stored: serde_json::Value = server
        .get("/api/v1/admin/config/stored")
        .add_header("Authorization", auth.clone())
        .await
        .json();
    assert_eq!(stored["stored"]["captcha_compression"], "70");
    assert_eq!(
        stored["shadowed"][0], "captcha_compression",
        "the overlay is a higher layer, so the stored value is not in effect"
    );

    let config: serde_json::Value = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", auth)
        .await
        .json();
    let entry = &config["config"]["captcha_compression"];
    assert_eq!(entry["value"], "90", "the override still wins");
    assert_eq!(entry["source"], "admin");
    assert_eq!(entry["shadowed_by"], "admin");
}

#[tokio::test]
async fn test_storing_a_value_an_override_masks_does_not_claim_it_is_in_effect() {
    // The sharpest form of the same bug: the response said "all are in effect" about a value
    // the server was demonstrably not using.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app());
    let auth = format!("Bearer {master_key}");

    server
        .patch("/api/v1/admin/config")
        .add_header("Authorization", auth.clone())
        .json(&json!({ "captcha_compression": 90 }))
        .await
        .assert_status_ok();

    let response = server
        .put("/api/v1/admin/config/stored")
        .add_header("Authorization", auth.clone())
        .json(&json!({ "captcha_compression": 70 }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("not in effect"),
        "the write must not claim an effect it did not have: {message}"
    );
    assert!(message.contains("captcha_compression"), "{message}");

    let config: serde_json::Value = server
        .get("/api/v1/admin/config")
        .add_header("Authorization", auth)
        .await
        .json();
    assert_eq!(config["config"]["captcha_compression"]["value"], "90");
}

#[tokio::test]
async fn test_storing_a_boot_field_still_reports_it_as_pending_a_restart() {
    // The other branch of the same message, kept honest: a stored boot field is not shadowed,
    // it is waiting, and the two must not be confused for one another.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();
    let server = TestServer::new(app.build_app());

    let response = server
        .put("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .json(&json!({ "server_port": 4321 }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("restart"), "{message}");
    assert!(!message.contains("not in effect"), "{message}");
    assert_eq!(body["pending_restart"][0], "server_port");
    assert!(
        body["shadowed"].as_array().unwrap().is_empty(),
        "nothing outranks the store here"
    );
}

#[tokio::test]
async fn test_a_row_the_configuration_drops_is_not_echoed_by_the_api() {
    // Only a hand-edit or a downgrade can produce such a row. `load` has always dropped it;
    // the API that displays the store used to read the raw table instead — and since every
    // secret is `Persist::Never`, that was also the one way a secret could be read back out.
    let app = TestApp::new().await;
    let master_key = app.master_key.clone();

    sqlx::query(
        "INSERT INTO config_settings (field, value, updated_at, updated_by)
         VALUES ('master_api_key', 'planted-secret-value', 0, 'hand-edit')",
    )
    .execute(app.storage.pool())
    .await
    .expect("plant the row");

    let server = TestServer::new(app.build_app());
    let stored: serde_json::Value = server
        .get("/api/v1/admin/config/stored")
        .add_header("Authorization", format!("Bearer {master_key}"))
        .await
        .json();

    assert!(
        stored["stored"]["master_api_key"].is_null(),
        "a row the configuration ignores must not appear here: {}",
        stored["stored"]
    );
    assert!(
        !serde_json::to_string(&stored)
            .unwrap()
            .contains("planted-secret-value"),
        "the value must not reach the response by any path"
    );
}
