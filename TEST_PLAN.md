# Test Coverage Improvement Plan

## Current Status

**Overall Coverage**: 78.38% (725/925 lines)
**Test Count**: 40 tests passing

---

## Coverage Analysis

### 🔴 Critical Gaps (0% Coverage)

1. **config.rs** - 37 lines, 0% coverage
   - **Criticality**: HIGH - App won't start without valid config
   - **Priority**: P0 (Immediate)

2. **tasks/cleanup.rs** - 13 lines, 0% coverage
   - **Criticality**: MEDIUM - Important for data management
   - **Priority**: P1 (High)

3. **main.rs** - 67 lines, 0% coverage
   - **Criticality**: LOW - Covered by integration tests
   - **Priority**: P3 (Optional)

### 🟠 Partial Coverage

4. **error.rs** - 38 lines, 44.74% coverage
   - **Criticality**: MEDIUM - Error handling paths
   - **Priority**: P1 (High)

5. **routes/sessions.rs** - 163 lines, 86.50% coverage
   - **Criticality**: MEDIUM - Core API, but already well-tested
   - **Priority**: P2 (Medium)

---

## Priority 0: Config Tests (IMMEDIATE)

### **File**: `src/config.rs`
**Current Coverage**: 0%
**Target Coverage**: 90%+
**Estimated Effort**: 2-3 hours

### Why Critical?
- Application startup depends on config
- Invalid config causes runtime panics
- Environment variable parsing bugs are hard to debug
- Security: validates sensitive keys exist

### Implementation Approach

**Design Change**: Instead of modifying environment variables in tests, we'll:
1. Create an `EnvProvider` trait for getting environment variables
2. Implement `RealEnv` for production (uses `std::env::var`)
3. Implement `MockEnv` for testing (uses HashMap)
4. Refactor `Config::from_env()` to accept an `EnvProvider`

This approach:
- ✅ Doesn't modify global state (environment variables)
- ✅ More testable and follows dependency injection
- ✅ Better design - SOLID principles
- ✅ No external dependencies needed

### Test Cases to Add

#### 1. **Valid Configuration Loading**
```rust
#[test]
fn test_config_from_valid_env() {
    let mut env = MockEnv::new();
    env.set("SERVER_HOST", "0.0.0.0");
    env.set("SERVER_PORT", "3000");
    env.set("DATABASE_URL", "sqlite::memory:");
    env.set("API_KEY_SALT", "test-salt");
    env.set("MASTER_API_KEY", "test-master");
    env.set("DEFAULT_SESSION_TTL_SECONDS", "300");
    env.set("MAX_SESSION_TTL_SECONDS", "3600");
    env.set("MAX_VALIDATION_ATTEMPTS", "3");
    env.set("CLEANUP_INTERVAL_SECONDS", "60");

    let config = Config::from_env_provider(&env).unwrap();
    assert_eq!(config.server_host, "0.0.0.0");
    assert_eq!(config.server_port, 3000);
    assert_eq!(config.api_key_salt, "test-salt");
}
```

#### 2. **Missing Required Environment Variables**
```rust
#[test]
fn test_config_missing_required_vars() {
    let mut env = MockEnv::new();
    // Don't set API_KEY_SALT
    env.set("SERVER_HOST", "0.0.0.0");
    env.set("MASTER_API_KEY", "test");

    let result = Config::from_env_provider(&env);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("API_KEY_SALT"));
}
```

#### 3. **Invalid Values**
```rust
#[test]
fn test_config_invalid_port() {
    let mut env = MockEnv::new();
    env.set_all_required();
    env.set("SERVER_PORT", "not-a-number");

    let result = Config::from_env_provider(&env);
    assert!(result.is_err());
}

#[test]
fn test_config_port_out_of_range() {
    let mut env = MockEnv::new();
    env.set_all_required();
    env.set("SERVER_PORT", "99999"); // > 65535

    let result = Config::from_env_provider(&env);
    assert!(result.is_err());
}
```

#### 4. **Default Values**
```rust
#[test]
fn test_config_uses_defaults() {
    let mut env = MockEnv::new();
    env.set_all_required();
    // Don't set DEFAULT_SESSION_TTL_SECONDS

    let config = Config::from_env_provider(&env).unwrap();
    assert_eq!(config.default_session_ttl_seconds, 300); // Default
}
```

#### 5. **server_address() Helper**
```rust
#[test]
fn test_server_address_format() {
    let mut env = MockEnv::new();
    env.set_all_required();
    env.set("SERVER_HOST", "127.0.0.1");
    env.set("SERVER_PORT", "8080");

    let config = Config::from_env_provider(&env).unwrap();
    assert_eq!(config.server_address(), "127.0.0.1:8080");
}
```

### Dependencies Needed
```toml
# No external dependencies needed!
# Using internal MockEnv implementation
```

### Implementation Steps
1. Create `EnvProvider` trait in `src/config.rs`
2. Implement `RealEnv` and `MockEnv`
3. Refactor `Config::from_env()` to use `EnvProvider`
4. Keep backward compatibility: `from_env()` uses `RealEnv`
5. Add `from_env_provider()` for testing
6. Create test module with MockEnv helpers
7. Add 10-12 test cases covering:
   - Valid configs
   - Missing required vars
   - Invalid values (port, numbers)
   - Default value application
   - Helper methods

**Expected Coverage**: 90-95%

---

## Priority 1: Cleanup Task Tests (HIGH)

### **File**: `src/tasks/cleanup.rs`
**Current Coverage**: 0%
**Target Coverage**: 85%+
**Estimated Effort**: 1-2 hours

### Why Important?
- Prevents database bloat
- Critical for long-running services
- Silent failures can cause issues
- Resource management edge cases

### Test Cases to Add

#### 1. **Cleanup Deletes Expired Sessions**
```rust
#[tokio::test]
async fn test_cleanup_removes_expired_sessions() {
    let storage = setup_test_storage().await;

    // Create expired session (expires_at in past)
    let expired = create_expired_session(&storage).await;

    // Create valid session (expires_at in future)
    let valid = create_valid_session(&storage).await;

    // Run cleanup once
    cleanup_expired_sessions(&storage).await;

    // Expired should be gone
    assert!(storage.get_session(&expired.id).await.unwrap().is_none());

    // Valid should still exist
    assert!(storage.get_session(&valid.id).await.unwrap().is_some());
}
```

#### 2. **Cleanup Handles Empty Database**
```rust
#[tokio::test]
async fn test_cleanup_with_no_sessions() {
    let storage = setup_test_storage().await;

    // Should not panic with empty DB
    let result = cleanup_expired_sessions(&storage).await;
    assert!(result.is_ok());
}
```

#### 3. **Cleanup Handles Database Errors**
```rust
#[tokio::test]
async fn test_cleanup_handles_db_error() {
    // Use closed/invalid connection
    let storage = create_invalid_storage().await;

    let result = cleanup_expired_sessions(&storage).await;
    // Should handle error gracefully, not panic
    assert!(result.is_err());
}
```

#### 4. **Cleanup Logs Correctly**
```rust
#[tokio::test]
async fn test_cleanup_logs_count() {
    // Use tracing-test to capture logs
    let storage = setup_test_storage().await;

    create_expired_session(&storage).await;
    create_expired_session(&storage).await;

    cleanup_expired_sessions(&storage).await;

    // Verify log contains "Cleaned up 2 expired sessions"
}
```

#### 5. **Start Cleanup Task (Integration)**
```rust
#[tokio::test]
async fn test_start_cleanup_task() {
    let storage = setup_test_storage().await;

    // Start with short interval for testing
    start_cleanup_task(storage.clone(), 1); // 1 second

    create_expired_session(&storage).await;

    // Wait for cleanup to run
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Should be cleaned up
    let count = storage.count_sessions().await.unwrap();
    assert_eq!(count, 0);
}
```

### Dependencies Needed
```toml
[dev-dependencies]
tracing-test = "0.2"  # For testing logs
```

### Implementation Steps
1. Extract cleanup logic to testable function
2. Add test helpers for creating expired sessions
3. Test cleanup execution, empty DB, errors
4. Test integration with task scheduler
5. Verify logging output

**Expected Coverage**: 85-90%

---

## Priority 1: Error Handling Tests (HIGH)

### **File**: `src/error.rs`
**Current Coverage**: 44.74%
**Target Coverage**: 80%+
**Estimated Effort**: 1 hour

### Why Important?
- User-facing error messages
- HTTP status code mapping
- Error logging behavior
- API contract compliance

### Test Cases to Add

#### 1. **Error Response Format**
```rust
#[test]
fn test_error_json_format() {
    let error = AppError::SessionNotFound;
    let response = error.into_response();

    let body = extract_json_body(response).await;

    assert_eq!(body["error"], "session_not_found");
    assert!(body["message"].is_string());
}
```

#### 2. **HTTP Status Codes**
```rust
#[test]
fn test_session_not_found_returns_404() {
    let error = AppError::SessionNotFound;
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn test_unauthorized_returns_401() {
    let error = AppError::Unauthorized("Invalid key".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn test_invalid_params_returns_400() {
    let error = AppError::InvalidSessionParams("Bad TTL".to_string());
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_database_error_returns_500() {
    let error = AppError::Database(sqlx::Error::PoolTimedOut);
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
```

#### 3. **Error Logging**
```rust
#[test]
fn test_database_error_logs() {
    // Use tracing-test
    let error = AppError::Database(sqlx::Error::PoolTimedOut);
    error.into_response();

    // Verify error was logged
}

#[test]
fn test_unauthorized_does_not_log() {
    // Client errors shouldn't spam logs
    let error = AppError::Unauthorized("test".to_string());
    error.into_response();

    // Verify no error log (only debug/trace)
}
```

#### 4. **Error Message Content**
```rust
#[test]
fn test_error_messages_are_safe() {
    // Should not leak internal details
    let error = AppError::Database(sqlx::Error::PoolTimedOut);
    let response = error.into_response();
    let body = extract_json_body(response).await;

    // Should be generic message, not raw SQL error
    assert!(!body["message"].as_str().unwrap().contains("SQL"));
}
```

### Implementation Steps
1. Add test helpers to extract response body
2. Test all error variants
3. Verify status codes match specification
4. Test error message safety (no info leaks)
5. Test logging behavior

**Expected Coverage**: 80-85%

---

## Priority 2: Session Routes Edge Cases (MEDIUM)

### **File**: `src/routes/sessions.rs`
**Current Coverage**: 86.50%
**Target Coverage**: 92%+
**Estimated Effort**: 2 hours

### Missing Coverage Areas

#### 1. **Concurrent Validation Attempts**
```rust
#[tokio::test]
async fn test_concurrent_validation_attempts() {
    // Test race condition: multiple validation attempts simultaneously
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app()).unwrap();

    let session = create_test_session(&server, &app.api_key).await;

    // Fire 3 validation requests concurrently
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let server = server.clone();
            let session_id = session.session_id.clone();
            tokio::spawn(async move {
                validate_session(&server, &session_id, "WRONG").await
            })
        })
        .collect();

    // All should complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Session should be deleted after 3 attempts
    let response = get_session(&server, &session.session_id).await;
    assert_eq!(response.status_code(), 404);
}
```

#### 2. **Image Retrieval During Validation**
```rust
#[tokio::test]
async fn test_get_image_during_validation() {
    // What happens if someone gets image while validation is happening?
    // Should work fine (no locks on read)
}
```

#### 3. **Boundary Values for Session Parameters**
```rust
#[tokio::test]
async fn test_create_session_max_ttl() {
    let app = TestApp::new().await;
    let server = TestServer::new(app.build_app()).unwrap();

    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", app.api_key))
        .json(&json!({
            "expires_in_seconds": 3600 // MAX_SESSION_TTL_SECONDS
        }))
        .await;

    assert_eq!(response.status_code(), 201);
}

#[tokio::test]
async fn test_create_session_exceeds_max_ttl() {
    let response = /* ... */
        .json(&json!({
            "expires_in_seconds": 3601 // Exceeds MAX
        }))
        .await;

    assert_eq!(response.status_code(), 400);
}
```

#### 4. **Expired Session Access**
```rust
#[tokio::test]
async fn test_validate_expired_session() {
    // Create session with 1 second TTL
    let session = create_session_with_ttl(&server, 1).await;

    // Wait for expiration
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Validation should fail
    let response = validate_session(&server, &session.id, "correct").await;
    assert_eq!(response.status_code(), 404);
}

#[tokio::test]
async fn test_get_image_expired_session() {
    // Similar test for image retrieval
}
```

#### 5. **Case Sensitivity in Validation**
```rust
#[tokio::test]
async fn test_validation_case_insensitive() {
    // Create session with solution "ABC123"
    let session = create_session_with_text(&server, "ABC123").await;

    // Validate with lowercase
    let response = validate(&server, &session.id, "abc123").await;
    assert_eq!(response.json()["valid"], true);

    // Validate with mixed case
    let response = validate(&server, &session.id, "AbC123").await;
    assert_eq!(response.json()["valid"], true);
}
```

#### 6. **Binary Image Caching Headers**
```rust
#[tokio::test]
async fn test_binary_image_cache_headers_near_expiration() {
    // Create session with 10 second TTL
    let session = create_session_with_ttl(&server, 10).await;

    // Get image immediately
    let response = get_binary_image(&server, &session.id).await;
    let max_age = extract_max_age(&response);

    // max-age should be close to 10 seconds
    assert!(max_age >= 8 && max_age <= 10);
}
```

### Implementation Steps
1. Add concurrent test helpers
2. Test boundary conditions (max TTL, min TTL)
3. Test expiration edge cases
4. Test case sensitivity thoroughly
5. Test cache header calculations
6. Test race conditions

**Expected Coverage**: 92-95%

---

## Priority 3: Main.rs (OPTIONAL)

### **File**: `src/main.rs`
**Current Coverage**: 0%
**Target Coverage**: N/A (Integration tested)
**Estimated Effort**: N/A

### Why Optional?
- Already integration tested via API tests
- Difficult to unit test (server startup)
- Low value: mostly glue code
- Tested indirectly by running `cargo run`

### If We Want to Test It

Would require:
1. Extracting testable functions from main
2. Creating a `run()` function that returns `Result`
3. Testing configuration parsing
4. Testing graceful shutdown

**Recommendation**: Skip for now. Coverage from integration tests is sufficient.

---

## Implementation Roadmap

### **Week 1: Critical Gaps**

**Day 1-2: Config Tests (P0)**
- [ ] Add `temp-env` dependency
- [ ] Write 12 config test cases
- [ ] Achieve 90%+ coverage on `config.rs`
- [ ] Verify all environment variables tested

**Day 3: Cleanup Tests (P1)**
- [ ] Extract testable cleanup function
- [ ] Add `tracing-test` dependency
- [ ] Write 5 cleanup test cases
- [ ] Test task scheduling

**Day 4: Error Tests (P1)**
- [ ] Add error response test helpers
- [ ] Write 8 error handling tests
- [ ] Verify all error variants covered
- [ ] Test logging behavior

### **Week 2: Refinement**

**Day 5-7: Session Routes Edge Cases (P2)**
- [ ] Add concurrent test helpers
- [ ] Write 10 edge case tests
- [ ] Test boundary conditions
- [ ] Test race conditions
- [ ] Achieve 92%+ on sessions

---

## Success Metrics

### **Phase 1 (Week 1) - Critical Coverage**

**Before**:
- Overall: 78.38%
- config.rs: 0%
- cleanup.rs: 0%
- error.rs: 44.74%

**After**:
- Overall: **85%+** ⭐
- config.rs: **90%+**
- cleanup.rs: **85%+**
- error.rs: **80%+**

### **Phase 2 (Week 2) - Comprehensive Coverage**

**Before**:
- Overall: 85%
- routes/sessions.rs: 86.50%

**After**:
- Overall: **88%+** ⭐⭐
- routes/sessions.rs: **92%+**

---

## Testing Best Practices

### 1. **Use Test Helpers**
```rust
// tests/common/mod.rs
pub async fn create_expired_session(storage: &StorageService) -> Session {
    let now = Utc::now().timestamp();
    Session::new(
        "TEST".to_string(),
        vec![1, 2, 3],
        0, // Already expired
        5, 220, 120, false
    )
}
```

### 2. **Test One Thing Per Test**
```rust
// BAD: Testing multiple things
#[test]
fn test_config() {
    let config = Config::from_env().unwrap();
    assert_eq!(config.port, 3000);
    assert_eq!(config.host, "0.0.0.0");
    assert!(config.api_key_salt.len() > 0);
}

// GOOD: Separate concerns
#[test]
fn test_config_port() {
    let config = Config::from_env().unwrap();
    assert_eq!(config.port, 3000);
}

#[test]
fn test_config_host() {
    let config = Config::from_env().unwrap();
    assert_eq!(config.host, "0.0.0.0");
}
```

### 3. **Use Descriptive Names**
```rust
// BAD
#[test]
fn test_1() { }

// GOOD
#[test]
fn test_config_missing_api_key_salt_returns_error() { }
```

### 4. **Test Error Cases**
Don't just test the happy path!
```rust
#[test]
fn test_valid_config() { /* ... */ }

#[test]
fn test_missing_required_var() { /* ... */ }

#[test]
fn test_invalid_port_format() { /* ... */ }
```

---

## Maintenance

### **After Adding Tests**

1. **Update Coverage Report**
```bash
cargo llvm-cov --all-features --workspace --html
open target/llvm-cov/html/index.html
```

2. **Update Documentation**
- Update `.claude/docs/TESTING.md` with new test patterns
- Document any new test helpers
- Add examples for future contributors

3. **CI/CD Integration**
```yaml
# .github/workflows/test.yml
- name: Run tests with coverage
  run: cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info

- name: Upload to Codecov
  uses: codecov/codecov-action@v3
  with:
    files: lcov.info
```

---

## Summary

**Priority Order**:
1. ⚠️ **P0**: Config tests (0% → 90%) - CRITICAL
2. 🔧 **P1**: Cleanup tests (0% → 85%) - HIGH
3. 🛡️ **P1**: Error tests (45% → 80%) - HIGH
4. 🎯 **P2**: Session edge cases (87% → 92%) - MEDIUM

**Expected Results**:
- Overall coverage: **78% → 88%+**
- All critical paths tested
- Production-ready test suite
- Clear test patterns for future work

**Time Investment**: ~10-12 hours total
**ROI**: High - Catches bugs before production, confidence in releases

---

**Created**: 2025-10-23
**Status**: Planning Phase
**Next Step**: Implement P0 (Config tests)
